//! Fixed collection capacity folding for type resolution.
//!
//! WHAT: resolves the narrow `ParsedCollectionCapacity` forms (integer literal or bare
//!       constant name) into a canonical `usize` capacity used by `TypeEnvironment`.
//! WHY: keeping capacity folding separate from the rest of type resolution lets
//!      `resolve_type.rs` focus on parsed-ref orchestration and diagnostic-type conversion,
//!      while this module owns the constant-evaluation boundary for collection sizes.
//!      Type syntax materialises literals at `Bits64` as the widest lossless carrier;
//!      this boundary owns the final range check against the compilation `Int` width.

use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidCollectionTypeReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::parsed::ParsedCollectionCapacity;
use crate::compiler_frontend::source::SourceSpan;
use moth_lexical::numeric::profile::{IntWidth, NumericProfile};

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
///      Capacity is a positive `Int` literal of the boundary width: header parsing has
///      no profile, so it materialises at `Bits64` and this boundary re-checks the value
///      against the compilation `IntWidth`. The literal's source text is not retained in
///      `ParsedCollectionCapacity::Literal`, so an out-of-width literal reports the closest
///      existing diagnostic, `CapacityOverflow`, rather than rebuilding an
///      `invalid_number_literal` diagnostic from unavailable text.
pub(crate) fn fold_collection_capacity(
    capacity: &ParsedCollectionCapacity,
    scope_context: Option<&ScopeContext>,
    type_environment: &mut TypeEnvironment,
    numeric_profile: NumericProfile,
) -> CollectionCapacityResult<usize> {
    let span = match capacity {
        ParsedCollectionCapacity::Literal { span, .. }
        | ParsedCollectionCapacity::BareConstant { span, .. } => *span,
    };

    let int_width = numeric_profile.int_width;

    match capacity {
        ParsedCollectionCapacity::Literal { value, .. } => {
            validate_capacity_value(*value, span, int_width)
        }

        ParsedCollectionCapacity::BareConstant { name, .. } => {
            let Some(scope_context) = scope_context else {
                return Err(CompilerDiagnostic::invalid_collection_type(
                    InvalidCollectionTypeReason::CapacityNotConstant,
                    span,
                )
                .into());
            };

            let Some(declaration) = scope_context.get_reference(name) else {
                return Err(CompilerDiagnostic::invalid_collection_type(
                    InvalidCollectionTypeReason::CapacityNotConstant,
                    span,
                )
                .into());
            };

            if !scope_context.is_explicit_compile_time_constant(declaration.as_declaration()) {
                return Err(CompilerDiagnostic::invalid_collection_type(
                    InvalidCollectionTypeReason::CapacityNotConstant,
                    span,
                )
                .into());
            }

            // Capacity syntax needs authored `#` provenance plus an already folded Int payload.
            // Template const classification cannot strengthen either part of that proof.
            if declaration.value.type_id != type_environment.builtins().int {
                return Err(CompilerDiagnostic::invalid_collection_type(
                    InvalidCollectionTypeReason::CapacityNotInt,
                    span,
                )
                .into());
            }

            let ExpressionKind::Int(value) = &declaration.value.kind else {
                return Err(CompilerDiagnostic::invalid_collection_type(
                    InvalidCollectionTypeReason::CapacityNotInt,
                    span,
                )
                .into());
            };

            validate_capacity_value(*value, span, int_width)
        }
    }
}
fn validate_capacity_value(
    value: i64,
    span: Option<SourceSpan>,
    int_width: IntWidth,
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
    // carrier, so a literal above the boundary `Int` width is rejected here with the
    // same overflow diagnostic the old `Int32` materialisation produced.
    if !int_width.contains(value) {
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
