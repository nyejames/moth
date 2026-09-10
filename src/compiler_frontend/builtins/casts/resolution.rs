//! AST cast resolver wiring.
//!
//! WHAT: resolves a parsed `cast` operand against an explicit typed boundary by
//!      selecting builtin, user-defined, or validation-only generic-bound evidence
//!      and validating the handling form. HIR and backend lowering consume the
//!      resolved `ExpressionKind::Cast` without re-solving trait evidence.
//! WHY: centralising evidence selection and cast-specific diagnostics prevents
//!      boundary callers from duplicating trait/evidence lookup logic.

use super::evidence::{builtin_evidence_fallibility, builtin_evidence_policy};
use super::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId, BuiltinCastTarget, builtin_cast_target_for_type,
};
use super::traits::{BUILTIN_CAST_TRAIT_ROWS, CoreCastTrait, builtin_cast_trait_metadata};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression_kind::ResolvedCastExpression;
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidCastReason};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_parameters::ActiveGenericTypeContext;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::traits::ids::TraitId;

/// Diagnostic result for resolved casts.
///
/// Cast resolution returns directly into the expression parser's diagnostic family, so both
/// owners share one error shape without changing accumulation or rendering boundaries.
type CastResolutionResult<T> = Result<T, CompilerDiagnostic>;

/// Inputs for resolving one explicit `cast` at a typed receiving boundary.
///
/// WHAT: groups the semantic boundary facts, trait evidence stores, and type
///      environment access needed to turn a parsed cast operand into a resolved
///      AST expression.
/// WHY: cast resolution is a stage-owned operation with several required
///      collaborators. Keeping them named prevents another long parser-to-AST
///      argument list from becoming the public shape of this boundary.
pub(crate) struct CastResolutionInput<'a> {
    pub(crate) source: Expression,
    pub(crate) target_type_id: TypeId,
    pub(crate) target: BuiltinCastTarget,
    pub(crate) requires_optional_wrap_after_cast: bool,
    pub(crate) handling: CastHandling,
    pub(crate) trait_environment: &'a TraitEnvironment,
    pub(crate) trait_evidence_environment: &'a TraitEvidenceEnvironment,
    pub(crate) type_environment: &'a mut TypeEnvironment,
    pub(crate) string_table: &'a StringTable,
    pub(crate) active_generic_type_context: Option<&'a ActiveGenericTypeContext>,
    pub(crate) span: Option<SourceSpan>,
}

/// Resolves a user-authored `cast` expression at an explicit typed boundary.
///
/// WHAT: validates the source/target pair, selects builtin, user-defined, or
///      validation-only generic-bound evidence, enforces the handling form, and
///      builds a resolved AST cast node.
/// WHY: the boundary owner already knows the target type; this function owns
///      evidence selection and user-facing cast diagnostics so callers do not
///      duplicate the lookup logic.
pub(crate) fn resolve_cast_expression(
    input: CastResolutionInput<'_>,
) -> CastResolutionResult<Expression> {
    let CastResolutionInput {
        source,
        target_type_id,
        target,
        requires_optional_wrap_after_cast,
        handling,
        trait_environment,
        trait_evidence_environment,
        type_environment,
        string_table,
        active_generic_type_context,
        span,
    } = input;

    let source_type_id = source.type_id;

    if type_environment.is_option(source_type_id) {
        return Err(CompilerDiagnostic::invalid_cast(
            InvalidCastReason::SourceIsOptional,
            Some(source_type_id),
            Some(target_type_id),
            source.span,
        ));
    }

    if source_type_id == target_type_id {
        return Err(CompilerDiagnostic::invalid_cast(
            InvalidCastReason::SameSourceAndTarget,
            Some(source_type_id),
            Some(target_type_id),
            source.span,
        ));
    }

    let source_target =
        builtin_cast_target_for_type(source_type_id, type_environment, string_table);

    let selection = select_cast_evidence(
        source_type_id,
        source_target,
        target,
        trait_environment,
        trait_evidence_environment,
        type_environment,
        active_generic_type_context,
    );

    let evidence = match handling {
        CastHandling::Infallible => match selection.infallible {
            Some(evidence) => evidence,
            None => {
                let reason = if selection.fallible.is_some() {
                    InvalidCastReason::FallibleEvidenceRequiresHandling
                } else {
                    InvalidCastReason::NoEvidence
                };
                return Err(CompilerDiagnostic::invalid_cast(
                    reason,
                    Some(source_type_id),
                    Some(target_type_id),
                    source.span,
                ));
            }
        },

        CastHandling::Propagate | CastHandling::Recover => match selection.fallible {
            Some(evidence) => evidence,
            None => {
                let reason = if selection.infallible.is_some() {
                    InvalidCastReason::InfallibleEvidenceCannotUseFallibleForm
                } else {
                    InvalidCastReason::NoEvidence
                };
                return Err(CompilerDiagnostic::invalid_cast(
                    reason,
                    Some(source_type_id),
                    Some(target_type_id),
                    source.span,
                ));
            }
        },
    };

    let cast = ResolvedCastExpression {
        source: Box::new(source),
        source_type_id,
        target_type_id,
        target,
        requires_optional_wrap_after_cast,
        evidence,
        handling,
        span,
    };

    let result_type_id = if cast.requires_optional_wrap_after_cast {
        type_environment.intern_option(cast.target_type_id)
    } else {
        cast.target_type_id
    };

    Ok(Expression::cast(cast, result_type_id, type_environment))
}

/// Evidence candidates for one source/target pair.
///
/// WHAT: holds the infallible and fallible evidence independently so the
///      handling-form validation can choose the right one and report the
///      opposite-form mismatch when applicable.
struct CastEvidenceSelection {
    infallible: Option<ResolvedCastEvidence>,
    fallible: Option<ResolvedCastEvidence>,
}

fn select_cast_evidence(
    source_type_id: TypeId,
    source_target: Option<BuiltinCastTarget>,
    target: BuiltinCastTarget,
    trait_environment: &TraitEnvironment,
    trait_evidence_environment: &TraitEvidenceEnvironment,
    type_environment: &TypeEnvironment,
    active_generic_type_context: Option<&ActiveGenericTypeContext>,
) -> CastEvidenceSelection {
    if let Some(source_target) = source_target
        && let Some((fallibility, policy)) = resolve_builtin_cast_target(source_target, target)
    {
        let evidence = ResolvedCastEvidence::Builtin { policy };
        return match fallibility {
            BuiltinCastFallibility::Infallible => CastEvidenceSelection {
                infallible: Some(evidence),
                fallible: None,
            },
            BuiltinCastFallibility::Fallible => CastEvidenceSelection {
                infallible: None,
                fallible: Some(evidence),
            },
        };
    }

    if let Some(selection) = generic_bound_evidence_for(
        source_type_id,
        target,
        trait_environment,
        type_environment,
        active_generic_type_context,
    ) {
        return selection;
    }

    let infallible = user_defined_evidence_for(
        source_type_id,
        target,
        BuiltinCastFallibility::Infallible,
        trait_environment,
        trait_evidence_environment,
    );

    let fallible = user_defined_evidence_for(
        source_type_id,
        target,
        BuiltinCastFallibility::Fallible,
        trait_environment,
        trait_evidence_environment,
    );

    CastEvidenceSelection {
        infallible,
        fallible,
    }
}

fn user_defined_evidence_for(
    source_type_id: TypeId,
    target: BuiltinCastTarget,
    fallibility: BuiltinCastFallibility,
    trait_environment: &TraitEnvironment,
    trait_evidence_environment: &TraitEvidenceEnvironment,
) -> Option<ResolvedCastEvidence> {
    let trait_kind = core_cast_trait_for_target_and_fallibility(target, fallibility)?;
    let trait_id = trait_environment
        .core_trait_id_for_static_name(builtin_cast_trait_metadata(trait_kind).trait_name)?;
    let evidence_id = trait_evidence_environment.canonical_for(source_type_id, trait_id)?;
    let evidence = trait_evidence_environment.get(evidence_id)?;
    let requirement = evidence.requirements.first()?;

    Some(ResolvedCastEvidence::UserDefined {
        evidence_id,
        method_path: requirement.method_path.clone(),
    })
}

/// Validation-only evidence selection for a generic parameter source inside a
/// generic function body.
///
/// WHAT: when the source type is an unresolved generic parameter and the active
///      context is a template-validation body (no substitutions), accept the
///      cast if the parameter declares the matching core cast trait bound.
/// WHY: generic function bodies are type-checked once before concrete instances
///      exist, so declaration-site bounds are the only available evidence.
///      Concrete instance emission supplies substitutions, reparses the body,
///      and selects real builtin or user-defined evidence instead.
fn generic_bound_evidence_for(
    source_type_id: TypeId,
    target: BuiltinCastTarget,
    trait_environment: &TraitEnvironment,
    type_environment: &TypeEnvironment,
    active_generic_type_context: Option<&ActiveGenericTypeContext>,
) -> Option<CastEvidenceSelection> {
    let parameter_id = generic_parameter_id_for_type(source_type_id, type_environment)?;
    let context = active_generic_type_context?;

    // Generic-bound evidence is only valid during template validation, where no
    // concrete substitutions exist. Concrete instance emission should have
    // already substituted the parameter away; if it somehow remains generic,
    // fall through to normal evidence selection rather than accepting a bound
    // that may not hold for the concrete type.
    if context.substitutions.is_some() {
        return None;
    }

    let bounds = type_environment.trait_bounds_for_generic_parameter(parameter_id)?;
    let mut infallible_trait_id: Option<TraitId> = None;
    let mut fallible_trait_id: Option<TraitId> = None;

    for trait_id in bounds {
        let Some((bound_target, bound_fallibility)) =
            builtin_cast_target_and_fallibility_for_trait_id(*trait_id, trait_environment)
        else {
            continue;
        };

        if bound_target != target {
            continue;
        }

        match bound_fallibility {
            BuiltinCastFallibility::Infallible => infallible_trait_id = Some(*trait_id),
            BuiltinCastFallibility::Fallible => fallible_trait_id = Some(*trait_id),
        }
    }

    Some(CastEvidenceSelection {
        infallible: infallible_trait_id.map(|trait_id| ResolvedCastEvidence::GenericBound {
            trait_id,
            parameter_id,
        }),
        fallible: fallible_trait_id.map(|trait_id| ResolvedCastEvidence::GenericBound {
            trait_id,
            parameter_id,
        }),
    })
}

fn generic_parameter_id_for_type(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> Option<GenericParameterId> {
    match type_environment.get(type_id) {
        Some(TypeDefinition::GenericParameter(parameter)) => Some(parameter.id),
        _ => None,
    }
}

fn builtin_cast_target_and_fallibility_for_trait_id(
    trait_id: TraitId,
    trait_environment: &TraitEnvironment,
) -> Option<(BuiltinCastTarget, BuiltinCastFallibility)> {
    match trait_environment.core_trait_kind(trait_id)? {
        crate::compiler_frontend::traits::environment::CoreTraitKind::Castable {
            target,
            fallibility,
        } => Some((target, fallibility)),
        crate::compiler_frontend::traits::environment::CoreTraitKind::Displayable => None,
    }
}

fn core_cast_trait_for_target_and_fallibility(
    target: BuiltinCastTarget,
    fallibility: BuiltinCastFallibility,
) -> Option<CoreCastTrait> {
    for metadata in BUILTIN_CAST_TRAIT_ROWS {
        if metadata.target == target && metadata.fallibility == fallibility {
            return Some(metadata.kind);
        }
    }
    None
}

/// Resolves a (source, target) pair to its fallibility and policy id, or
/// `None` when no initial builtin evidence row exists.
pub(crate) fn resolve_builtin_cast_target(
    source: BuiltinCastTarget,
    target: BuiltinCastTarget,
) -> Option<(BuiltinCastFallibility, BuiltinCastPolicyId)> {
    let fallibility = builtin_evidence_fallibility(source, target)?;
    let policy = builtin_evidence_policy(source, target)?;
    Some((fallibility, policy))
}
