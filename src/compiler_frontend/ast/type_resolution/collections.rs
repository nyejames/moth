//! Fixed collection capacity folding for type resolution.
//!
//! WHAT: resolves the narrow `ParsedCollectionCapacity` forms (integer literal or bare
//!       constant name) into a canonical `usize` capacity used by `TypeEnvironment`.
//! WHY: keeping capacity folding separate from the rest of type resolution lets
//!      `resolve_type.rs` focus on parsed-ref orchestration and diagnostic-type conversion,
//!      while this module owns the constant-evaluation boundary for collection sizes.
//!      Type syntax materialises literals at `Bits64` as the widest lossless carrier;
//!      this boundary owns the final range check against the exact `I32` capacity domain.

use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::module_ast::scope_context::{ScopeContext, ScopeDeclarationRef};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidCollectionTypeReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::parsed::ParsedCollectionCapacity;
use crate::compiler_frontend::source::SourceSpan;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

pub(crate) type CollectionCapacityResult<T> = Result<T, CollectionCapacityDiagnostic>;

pub(crate) struct CollectionCapacityDiagnostic(CompilerDiagnostic);

impl CollectionCapacityDiagnostic {
    pub(crate) fn as_diagnostic(&self) -> &CompilerDiagnostic {
        &self.0
    }

    pub(crate) fn into_diagnostic(self) -> CompilerDiagnostic {
        self.0
    }
}

impl From<CompilerDiagnostic> for CollectionCapacityDiagnostic {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        Self(diagnostic)
    }
}

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
) -> CollectionCapacityResult<usize> {
    let span = match capacity {
        ParsedCollectionCapacity::Literal { span, .. }
        | ParsedCollectionCapacity::BareConstant { span, .. } => *span,
    };

    match capacity {
        ParsedCollectionCapacity::Literal { value, .. } => validate_capacity_value(*value, span),

        ParsedCollectionCapacity::BareConstant { name, .. } => {
            let Some(scope_context) = scope_context else {
                return Err(CompilerDiagnostic::invalid_collection_type(
                    InvalidCollectionTypeReason::CapacityNotConstant,
                    span,
                )
                .into());
            };

            let Some(declaration_ref) = scope_context.get_reference(name) else {
                return Err(CompilerDiagnostic::invalid_collection_type(
                    InvalidCollectionTypeReason::CapacityNotConstant,
                    span,
                )
                .into());
            };

            let declaration = match &declaration_ref {
                ScopeDeclarationRef::Local { declaration, .. } => declaration.as_ref(),
                ScopeDeclarationRef::Shared(declaration) => declaration,
                // An unresolved body-local numeric binding has no declaration or folded value
                // yet, so it can never satisfy the capacity constant contract.
                ScopeDeclarationRef::PendingNumeric { .. } => {
                    return Err(CompilerDiagnostic::invalid_collection_type(
                        InvalidCollectionTypeReason::CapacityNotConstant,
                        span,
                    )
                    .into());
                }
            };

            if !scope_context.is_explicit_compile_time_constant(declaration) {
                return Err(CompilerDiagnostic::invalid_collection_type(
                    InvalidCollectionTypeReason::CapacityNotConstant,
                    span,
                )
                .into());
            }

            // Capacity syntax needs authored `#` provenance plus an already folded exact `I32`
            // payload. Template const classification cannot strengthen either part of that proof.
            if type_environment.fixed_scalar(declaration.value.type_id) != Some(FixedScalar::I32) {
                return Err(CompilerDiagnostic::invalid_collection_type(
                    InvalidCollectionTypeReason::CapacityNotInt,
                    span,
                )
                .into());
            }

            let ExpressionKind::FixedScalar(value) = &declaration.value.kind else {
                return Err(CompilerDiagnostic::invalid_collection_type(
                    InvalidCollectionTypeReason::CapacityNotInt,
                    span,
                )
                .into());
            };

            let Some(value) = value.as_i64() else {
                return Err(CompilerDiagnostic::invalid_collection_type(
                    InvalidCollectionTypeReason::CapacityNotInt,
                    span,
                )
                .into());
            };

            validate_capacity_value(value, span)
        }
    }
}

fn validate_capacity_value(
    value: i64,
    span: Option<SourceSpan>,
) -> CollectionCapacityResult<usize> {
    if value < 0 {
        return Err(CompilerDiagnostic::invalid_collection_type(
            InvalidCollectionTypeReason::NegativeCapacity,
            span,
        )
        .into());
    }

    if value == 0 {
        return Err(CompilerDiagnostic::invalid_collection_type(
            InvalidCollectionTypeReason::ZeroCapacity,
            span,
        )
        .into());
    }

    // Header parsing materialises capacity literals at `Bits64` as the widest lossless
    // carrier, so a literal above the exact `I32` capacity domain is rejected here with the
    // closest existing overflow diagnostic.
    if FixedScalarValue::signed(FixedScalar::I32, value).is_none() {
        return Err(CompilerDiagnostic::invalid_collection_type(
            InvalidCollectionTypeReason::CapacityOverflow,
            span,
        )
        .into());
    }

    usize::try_from(value).map_err(|_| {
        CompilerDiagnostic::invalid_collection_type(
            InvalidCollectionTypeReason::CapacityOverflow,
            span,
        )
        .into()
    })
}
