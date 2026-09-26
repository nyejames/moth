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
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceVariantPayloadDefinition, TypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{BuiltinTypeKey, TypeId};
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::ids::BlockId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::HirNumericOperands;
use crate::compiler_frontend::hir::patterns::HirPattern;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::reachability::{
    HirBackendSelection, HirReachability, ReachableAssertionMessageUse,
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
///       type checked but do not block the experimental Wasm build path. Fixed-width numeric and
///       `Byte` values have no lowering on either target yet, so both targets reject reachable
///       values of those types before any target-specific operation check.
/// WHY: fail early with a structured Rule error carrying the source span instead of a vague
///      backend-internal lowering failure.
pub fn validate_hir_backend_feature_support(
    input: BackendFeatureValidationInput<'_>,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let reachability = input.reachability;

    // A fixed-width scalar is a type-shape gap rather than an operation gap, so this gate runs
    // before every other check on both targets and fixes the reason deterministically: a value whose
    // type carries one reports `FixedWidthScalarValues` even when the same code also triggers an
    // operation-shaped reason such as a cross-module call or a generic instance.
    validate_fixed_width_scalar_values(
        input.hir,
        input.type_environment,
        reachability.backend_selection(),
        input.target,
        string_table,
    )?;

    match input.target {
        BackendTarget::Wasm => {
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

/// Reports the first reachable fixed-width numeric or `Byte` value for either target.
///
/// WHAT: `I8`..`U64`, `F16`..`F64` and `Byte` are canonical frontend identities, but no target has
///       a runtime representation or lowering for values of those types yet. A reachable local,
///       expression or function signature whose type carries one is rejected.
/// WHY: reject early with a structured unsupported-backend diagnostic instead of a backend-internal
///      lowering failure. Unreachable private helpers keep their signatures and stay valid typed
///      HIR.
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
    let mut fixed_scalar_types = FixedScalarTypes::new(type_environment);

    // Reachable blocks come first so the diagnostic lands on the authored local or expression. The
    // signature pass then covers generated or synthetic values that carry a fixed scalar with no
    // reachable expression to place a span on.
    let occurrence = first_unsupported_module_occurrence(hir, selection.blocks(), &mut |type_id| {
        fixed_scalar_types.contains(type_id)
    })
    .or_else(|| {
        first_unsupported_function_signature_occurrence(hir, selection, &mut |type_id| {
            fixed_scalar_types.contains(type_id)
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
//  Fixed-width scalar type structure
// -----------------------------------------------------------------------------

/// Memoised semantic predicate for fixed-width numeric and `Byte` type structure.
///
/// WHAT: answers whether a canonical type is one of `I8`..`U64`, `F16`..`F64` or `Byte`, or carries
///       one through options, collections, maps, fallible carriers, tuples, struct fields, choice
///       payloads, generic instances or function types.
/// WHY: the predicate is transitive over type structure and every reachable value asks about its
///      own type, so answers are memoised per `TypeId` and recursive nominal types are broken with
///      a visiting set instead of a fresh structural walk per expression.
struct FixedScalarTypes<'environment> {
    type_environment: &'environment TypeEnvironment,
    memo: FxHashMap<TypeId, bool>,
    visiting: FxHashSet<TypeId>,
}

impl<'environment> FixedScalarTypes<'environment> {
    fn new(type_environment: &'environment TypeEnvironment) -> Self {
        Self {
            type_environment,
            memo: FxHashMap::default(),
            visiting: FxHashSet::default(),
        }
    }

    fn contains(&mut self, type_id: TypeId) -> bool {
        self.visit(type_id).found
    }

    /// Walks one type and reports whether it carries a fixed scalar.
    ///
    /// WHAT: `open_cycle` marks an answer that passed through a type still being visited.
    /// WHY: a recursive nominal type's answer is only final once its own walk completes, so a
    ///      provisional `false` must not be memoised or a fixed scalar reachable through the cycle
    ///      would be hidden from every later query.
    fn visit(&mut self, type_id: TypeId) -> FixedScalarVisit {
        if let Some(found) = self.memo.get(&type_id) {
            return FixedScalarVisit {
                found: *found,
                open_cycle: false,
            };
        }

        if !self.visiting.insert(type_id) {
            return FixedScalarVisit {
                found: false,
                open_cycle: true,
            };
        }

        let visit = self.walk(type_id);
        self.visiting.remove(&type_id);

        if !visit.open_cycle {
            self.memo.insert(type_id, visit.found);
        }

        visit
    }

    fn walk(&mut self, type_id: TypeId) -> FixedScalarVisit {
        // Copy the environment reference out first so the borrowed definition does not borrow
        // `self` while the walk recurses.
        let type_environment = self.type_environment;
        let Some(definition) = type_environment.get(type_id) else {
            return FixedScalarVisit::NOT_FOUND;
        };

        let mut visit = FixedScalarVisit::NOT_FOUND;
        match definition {
            TypeDefinition::Builtin(builtin) => {
                visit.found = matches!(builtin.key, BuiltinTypeKey::FixedScalar(_));
            }

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

            // Collections, maps, options, fallible carriers and tuples keep their element types as
            // constructor arguments.
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

            // Instance arguments cover `Box of Byte`; the base nominal covers fixed scalars in
            // declaration fields that the arguments do not substitute, such as
            // `Box type T = | raw {U8} |`.
            TypeDefinition::GenericInstance(instance) => {
                for argument in instance.arguments.iter() {
                    visit.merge(self.visit(*argument));
                }

                if let Some(base_type_id) = type_environment.type_id_for_nominal_id(instance.base) {
                    visit.merge(self.visit(base_type_id));
                }
            }

            // Opaque host types, generic parameters and the compile-time-only const-record marker
            // never carry a fixed-width scalar.
            TypeDefinition::External(_)
            | TypeDefinition::GenericParameter(_)
            | TypeDefinition::AnonymousConstRecordMarker => {}
        }

        visit
    }
}

/// One type-structure walk result for the fixed-width scalar predicate.
struct FixedScalarVisit {
    found: bool,
    open_cycle: bool,
}

impl FixedScalarVisit {
    const NOT_FOUND: Self = Self {
        found: false,
        open_cycle: false,
    };

    fn merge(&mut self, other: Self) {
        self.found |= other.found;
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
