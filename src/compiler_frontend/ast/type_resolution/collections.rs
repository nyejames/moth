//! Fixed collection capacity folding for type resolution.
//!
//! WHAT: resolves the narrow `ParsedCollectionCapacity` forms (integer literal or bare
//!       constant name) into a canonical `usize` capacity used by `TypeEnvironment`.
//! WHY: keeping capacity folding separate from the rest of type resolution lets
//!      `resolve_type.rs` focus on parsed-ref orchestration and diagnostic-type conversion,
//!      while this module owns the constant-evaluation boundary for collection sizes.

use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidCollectionTypeReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::parsed::ParsedCollectionCapacity;
use crate::compiler_frontend::source::SourceSpan;

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

            validate_capacity_value(*value, span)
        }
    }
}
fn validate_capacity_value(
    value: i32,
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

    usize::try_from(value).map_err(|_| {
        CompilerDiagnostic::invalid_collection_type(
            InvalidCollectionTypeReason::CapacityOverflow,
            span,
        )
        .into()
    })
}
