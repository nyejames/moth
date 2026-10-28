//! Fixed collection capacity folding for type resolution.
//!
//! WHAT: resolves the narrow `ParsedCollectionCapacity` forms (integer literal or bare
//!       constant name) into a canonical `usize` capacity used by `TypeEnvironment`.
//! WHY: keeping capacity folding separate from the rest of type resolution lets
//!      `resolve_type.rs` focus on parsed-ref orchestration and diagnostic-type conversion,
//!      while this module owns the constant-evaluation boundary for collection sizes.
//!      Type syntax materialises literals at `Bits64` as the widest lossless carrier;
//!      this boundary owns the final range check against the exact `I32` capacity domain.

use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::module_ast::scope_context::{ScopeContext, ValueDeclarationRef};
use crate::compiler_frontend::ast::type_resolution::context::TypeResolutionFailureKind;
use crate::compiler_frontend::compiler_messages::InvalidCollectionTypeReason;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::parsed::ParsedCollectionCapacity;
use crate::compiler_frontend::source::SourceSpan;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

pub(crate) type CollectionCapacityResult<T> = Result<T, DiagnosticDraft>;

/// Fold a parsed collection capacity into a canonical `usize`.
///
/// WHAT: resolves the narrow `ParsedCollectionCapacity` forms (integer literal or bare
///       constant name) directly without invoking the full expression parser.
/// WHY: the parser already rejected general capacity forms, so type resolution only
///      needs to validate literals and look up bare constants in the visible scope.
///      Capacity is a positive `I32` value: header parsing has no resolved environment, so it
///      materialises the literal at `Bits64` and this boundary re-checks the value against the
///      exact `I32` capacity domain. The literal's source text is not retained in
///      `ParsedCollectionCapacity::Literal`, so an out-of-domain literal reports the closest
///      existing diagnostic, `CapacityOverflow`, rather than rebuilding an
///      `invalid_number_literal` diagnostic from unavailable text.
pub(crate) fn fold_collection_capacity(
    capacity: &ParsedCollectionCapacity,
    scope_context: Option<&ScopeContext>,
    type_environment: &mut TypeEnvironment,
    const_values: &ConstValueStore,
) -> CollectionCapacityResult<usize> {
    let mut failure_kind = TypeResolutionFailureKind::Other;
    fold_collection_capacity_with_failure_kind(
        capacity,
        scope_context,
        type_environment,
        const_values,
        &mut failure_kind,
    )
}

pub(crate) fn fold_collection_capacity_with_failure_kind(
    capacity: &ParsedCollectionCapacity,
    scope_context: Option<&ScopeContext>,
    type_environment: &mut TypeEnvironment,
    const_values: &ConstValueStore,
    failure_kind: &mut TypeResolutionFailureKind,
) -> CollectionCapacityResult<usize> {
    let span = match capacity {
        ParsedCollectionCapacity::Literal { span, .. }
        | ParsedCollectionCapacity::BareConstant { span, .. } => *span,
    };

    match capacity {
        ParsedCollectionCapacity::Literal { value, .. } => validate_capacity_value(*value, span),

        ParsedCollectionCapacity::BareConstant { name, .. } => {
            let Some(scope_context) = scope_context else {
                return Err(capacity_not_constant(failure_kind, span));
            };

            let Some(declaration_ref) = scope_context.get_value_reference(name, span)? else {
                return Err(capacity_not_constant(failure_kind, span));
            };

            let declaration = match &declaration_ref {
                ValueDeclarationRef::Local { declaration, .. } => declaration.as_ref(),
                ValueDeclarationRef::Shared(declaration) => declaration,
                // An unresolved body-local numeric binding has no declaration or folded value
                // yet, so it can never satisfy the capacity constant contract.
                ValueDeclarationRef::PendingNumeric { .. } => {
                    return Err(capacity_not_constant(failure_kind, span));
                }
            };

            if !scope_context.is_explicit_compile_time_constant(declaration) {
                return Err(capacity_not_constant(failure_kind, span));
            }
            // Capacity syntax needs authored `#` provenance plus an already folded exact `I32`
            // payload. Template const classification cannot strengthen either part of that proof.
            if type_environment.fixed_scalar(declaration.type_id) != Some(FixedScalar::I32) {
                return Err(calls_data::invalid_collection_type(
                    InvalidCollectionTypeReason::CapacityNotInt,
                    span,
                ));
            }

            let Some(ConstValuePayload::FixedScalar(value)) = const_values
                .value_for_path(&declaration.id)
                .and_then(|id| const_values.payload(id))
            else {
                return Err(calls_data::invalid_collection_type(
                    InvalidCollectionTypeReason::CapacityNotInt,
                    span,
                ));
            };

            let Some(value) = value.as_i64() else {
                return Err(calls_data::invalid_collection_type(
                    InvalidCollectionTypeReason::CapacityNotInt,
                    span,
                ));
            };

            validate_capacity_value(value, span)
        }
    }
}

fn capacity_not_constant(
    failure_kind: &mut TypeResolutionFailureKind,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    *failure_kind = TypeResolutionFailureKind::CapacityNotConstant;
    calls_data::invalid_collection_type(InvalidCollectionTypeReason::CapacityNotConstant, span)
}

fn validate_capacity_value(
    value: i64,
    span: Option<SourceSpan>,
) -> CollectionCapacityResult<usize> {
    if value < 0 {
        return Err(calls_data::invalid_collection_type(
            InvalidCollectionTypeReason::NegativeCapacity,
            span,
        ));
    }

    if value == 0 {
        return Err(calls_data::invalid_collection_type(
            InvalidCollectionTypeReason::ZeroCapacity,
            span,
        ));
    }

    // Header parsing materialises capacity literals at `Bits64` as the widest lossless
    // carrier, so a literal above the exact `I32` capacity domain is rejected here with the
    // closest existing overflow diagnostic.
    if FixedScalarValue::signed(FixedScalar::I32, value).is_none() {
        return Err(calls_data::invalid_collection_type(
            InvalidCollectionTypeReason::CapacityOverflow,
            span,
        ));
    }

    usize::try_from(value).map_err(|_| {
        calls_data::invalid_collection_type(InvalidCollectionTypeReason::CapacityOverflow, span)
    })
}
