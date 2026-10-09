//! AST cast resolver wiring.
//!
//! WHAT: resolves a parsed `cast` operand against an explicit typed boundary by
//!      selecting builtin, user-defined, or validation-only generic-bound evidence,
//!      instantiating generic evidence methods, and validating the handling form.
//!      HIR and backend lowering consume the resolved `ExpressionKind::Cast` without
//!      re-solving trait evidence.
//! WHY: centralising evidence selection and cast-specific diagnostics prevents
//!      boundary callers from duplicating trait/evidence lookup logic, and keeps the
//!      "evidence resolves before HIR" contract in one stage owner.

use super::evidence::lookup_builtin_evidence;
use super::targets::{BuiltinCastFallibility, BuiltinCastTarget, builtin_cast_target_for_type};
use super::traits::{builtin_cast_trait_metadata, core_cast_trait_for_target_and_fallibility};
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression_kind::ResolvedCastExpression;
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::ast::generic_functions::instantiate_generic_cast_evidence_method;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidCastReason};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_parameters::ActiveGenericTypeContext;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::traits::ids::{TraitEvidenceId, TraitId};
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::profile::NumericProfile;

/// Diagnostic result for resolved casts.
///
/// Cast resolution returns into the expression parser's diagnostic family, which carries both
/// user diagnostics and genuine infrastructure failures, so generic evidence instantiation can
/// report its own diagnostics through the same boundary.
type CastResolutionResult<T> = Result<T, ExpressionParseError>;

/// Inputs for resolving one explicit `cast` at a typed receiving boundary.
///
/// WHAT: groups the semantic boundary facts, trait evidence stores, and body-emission
///      access needed to turn a parsed cast operand into a resolved AST expression.
/// WHY: cast resolution is a stage-owned operation with several required
///      collaborators. Keeping them named prevents another long parser-to-AST
pub(crate) struct CastResolutionInput<'a, 'interner> {
    pub(crate) source: Expression,
    pub(crate) target_type_id: TypeId,
    pub(crate) target: BuiltinCastTarget,
    pub(crate) requires_optional_wrap_after_cast: bool,
    pub(crate) handling: CastHandling,
    pub(crate) numeric_profile: NumericProfile,
    pub(crate) scope_context: &'a ScopeContext,
    pub(crate) trait_environment: &'a TraitEnvironment,
    pub(crate) trait_evidence_environment: &'a TraitEvidenceEnvironment,
    pub(crate) type_interner: &'a mut AstTypeInterner<'interner>,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork: &'a mut PathInternerFork,
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
    input: CastResolutionInput<'_, '_>,
) -> CastResolutionResult<Expression> {
    let CastResolutionInput {
        source,
        target_type_id,
        target,
        requires_optional_wrap_after_cast,
        handling,
        numeric_profile,
        scope_context,
        trait_environment,
        trait_evidence_environment,
        type_interner,
        string_table,
        path_fork,
        active_generic_type_context,
        span,
    } = input;

    let source_type_id = source.type_id;
    let source_span = source.span;

    if type_interner.environment().is_option(source_type_id) {
        return Err(CompilerDiagnostic::invalid_cast(
            InvalidCastReason::SourceIsOptional,
            Some(source_type_id),
            Some(target_type_id),
            source_span,
        )
        .into());
    }

    if source_type_id == target_type_id {
        return Err(CompilerDiagnostic::invalid_cast(
            InvalidCastReason::SameSourceAndTarget,
            Some(source_type_id),
            Some(target_type_id),
            source_span,
        )
        .into());
    }

    let builtin_row = builtin_cast_target_for_type(
        source_type_id,
        type_interner.environment(),
        string_table,
        path_fork,
    )
    .and_then(|source_target| lookup_builtin_evidence(source_target, target, numeric_profile));

    let (evidence, fallibility) = if let Some(row) = builtin_row {
        (
            ResolvedCastEvidence::Builtin { policy: row.policy },
            row.fallibility,
        )
    } else {
        let selection = select_cast_evidence(
            source_type_id,
            target,
            trait_environment,
            trait_evidence_environment,
            type_interner.environment(),
            scope_context,
            active_generic_type_context,
        );
        match (selection.infallible, selection.fallible) {
            (Some(evidence), None) => (evidence, BuiltinCastFallibility::Infallible),
            (None, Some(evidence)) => (evidence, BuiltinCastFallibility::Fallible),
            // Core evidence registration forbids both classes for one pair. Treat an absent or
            // contradictory pair as unavailable rather than coupling selection to cast spelling.
            (None, None) | (Some(_), Some(_)) => {
                return Err(CompilerDiagnostic::invalid_cast(
                    InvalidCastReason::NoEvidence,
                    Some(source_type_id),
                    Some(target_type_id),
                    source_span,
                )
                .into());
            }
        }
    };

    let (evidence, source) = instantiate_user_defined_evidence_method(
        evidence,
        source,
        source_span,
        scope_context,
        type_interner,
        string_table,
        path_fork,
    )?;

    let cast = ResolvedCastExpression {
        source: Box::new(source),
        source_type_id,
        target_type_id,
        target,
        requires_optional_wrap_after_cast,
        evidence,
        fallibility,
        handling,
        span,
    };

    let result_type_id = if cast.requires_optional_wrap_after_cast {
        type_interner
            .environment_mut_for_derived_types()
            .intern_option(cast.target_type_id)
    } else {
        cast.target_type_id
    };

    Ok(Expression::cast(
        cast,
        result_type_id,
        type_interner.environment(),
    ))
}

/// Resolves one selected user-defined evidence method to its concrete instance.
///
/// WHAT: when the selected evidence method names a generic receiver template, infers and
///       validates the template's concrete instance through the shared receiver-method owner,
///       records its materialisation request, and returns the generated instance path together
///       with the cast operand moved back out of the hidden call's inference argument.
/// WHY: a cast's evidence method is a hidden receiver call. Evidence resolves before HIR, so the
///      template origin must be replaced by its concrete instance here; the operand keeps one
///      authored evaluation and one AST allocation across inference and the final cast node.
fn instantiate_user_defined_evidence_method(
    evidence: ResolvedCastEvidence,
    source: Expression,
    source_span: Option<SourceSpan>,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> CastResolutionResult<(ResolvedCastEvidence, Expression)> {
    let ResolvedCastEvidence::UserDefined {
        evidence_id,
        method_path,
    } = evidence
    else {
        return Ok((evidence, source));
    };

    // Exact same-file and projected conformance evidence names its concrete method directly.
    // Constructor fallback selection only accepts generic receiver templates, so a template is
    // the only case that still has to become a concrete callable before HIR.
    let Some(template) = scope_context.lookup_generic_function_template(&method_path) else {
        return Ok((
            ResolvedCastEvidence::UserDefined {
                evidence_id,
                method_path,
            },
            source,
        ));
    };

    let (instance_path, source) = instantiate_generic_cast_evidence_method(
        template,
        source,
        source_span,
        scope_context,
        type_interner,
        string_table,
        path_fork,
    )?;

    Ok((
        ResolvedCastEvidence::UserDefined {
            evidence_id,
            method_path: instance_path,
        },
        source,
    ))
}

/// Evidence candidates for one source/target pair.
///
/// WHAT: holds both evidence classes independently so plain cast selection follows the
///       resolved source-target pair rather than a handling spelling.
struct CastEvidenceSelection {
    infallible: Option<ResolvedCastEvidence>,
    fallible: Option<ResolvedCastEvidence>,
}

fn select_cast_evidence(
    source_type_id: TypeId,
    target: BuiltinCastTarget,
    trait_environment: &TraitEnvironment,
    trait_evidence_environment: &TraitEvidenceEnvironment,
    type_environment: &TypeEnvironment,
    scope_context: &ScopeContext,
    active_generic_type_context: Option<&ActiveGenericTypeContext>,
) -> CastEvidenceSelection {
    // `Byte` and `Dec` conversions are compiler-owned only: valid pairs already resolved
    // through the builtin row above, and an absent row must not fall back to a source-authored
    // or generic-bound conversion. Fixed-width integer and float targets are ordinary
    // catalogue targets, so user-defined and generic-bound evidence may supply the conversion.
    if matches!(
        target,
        BuiltinCastTarget::Fixed(FixedScalar::Byte) | BuiltinCastTarget::Number(_)
    ) || type_environment.number_scale(source_type_id).is_some()
    {
        return CastEvidenceSelection {
            infallible: None,
            fallible: None,
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
        type_environment,
        scope_context,
    );

    let fallible = user_defined_evidence_for(
        source_type_id,
        target,
        BuiltinCastFallibility::Fallible,
        trait_environment,
        trait_evidence_environment,
        type_environment,
        scope_context,
    );

    CastEvidenceSelection {
        infallible,
        fallible,
    }
}

/// Selects registered user-defined evidence for one source/target/fallibility triple.
///
/// WHAT: takes the exact registered evidence for the source type, else the evidence registered on
///       the nominal constructor of a generic nominal instance source.
/// WHY: core cast traits resolve before HIR, and a generic nominal constructor conformance is the
///      declared evidence for every instance of that constructor. The constructor retry mirrors
///      the generic-bound lookup so both consumers read one stable evidence model.
fn user_defined_evidence_for(
    source_type_id: TypeId,
    target: BuiltinCastTarget,
    fallibility: BuiltinCastFallibility,
    trait_environment: &TraitEnvironment,
    trait_evidence_environment: &TraitEvidenceEnvironment,
    type_environment: &TypeEnvironment,
    scope_context: &ScopeContext,
) -> Option<ResolvedCastEvidence> {
    let trait_kind = core_cast_trait_for_target_and_fallibility(target, fallibility)?;
    let trait_id = trait_environment
        .core_trait_id_for_static_name(builtin_cast_trait_metadata(trait_kind).trait_name)?;
    let evidence_id = trait_evidence_environment
        .canonical_for(source_type_id, trait_id)
        .or_else(|| {
            constructor_evidence_for_instance(
                source_type_id,
                trait_id,
                type_environment,
                trait_evidence_environment,
                scope_context,
            )
        })?;
    let evidence = trait_evidence_environment.get(evidence_id)?;
    let requirement = evidence.requirements.first()?;

    Some(ResolvedCastEvidence::UserDefined {
        evidence_id,
        method_path: requirement.method_path,
    })
}

/// Resolves constructor-registered evidence for a generic nominal instance source.
///
/// WHAT: retries the exact evidence lookup on the instance's nominal constructor, and keeps the
///       constructor row only when the conforming method is a visible generic receiver template.
/// WHY: a constructor conformance is aligned with one instance only when its method is generic
///      over the constructor's parameters, so it can be instantiated for the actual source type.
///      A concrete receiver method declared for one sibling instance cannot serve another
///      instance, and must stay unselected exactly as it was before this retry existed.
fn constructor_evidence_for_instance(
    source_type_id: TypeId,
    trait_id: TraitId,
    type_environment: &TypeEnvironment,
    trait_evidence_environment: &TraitEvidenceEnvironment,
    scope_context: &ScopeContext,
) -> Option<TraitEvidenceId> {
    let Some(TypeDefinition::GenericInstance(instance)) = type_environment.get(source_type_id)
    else {
        return None;
    };
    let base_type_id = type_environment.type_id_for_nominal_id(instance.base)?;
    let evidence_id = trait_evidence_environment.canonical_for(base_type_id, trait_id)?;
    let evidence = trait_evidence_environment.get(evidence_id)?;
    let requirement = evidence.requirements.first()?;
    scope_context.lookup_generic_function_template(&requirement.method_path)?;

    Some(evidence_id)
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
