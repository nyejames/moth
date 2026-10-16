//! Lazy nominal generic instance materialization for AST type resolution.
//!
//! WHAT: materializes concrete `TypeId`s for user-declared generic structs and choices once
//!       their base path and resolved type arguments are known, then validates declaration-site
//!       trait-bound evidence for the interned instance.
//! WHY: generic nominal instantiation interns canonical argument identities in `TypeEnvironment`
//!      and runs static bound checks.
//!
//! This module owns:
//! - lazy interning of generic struct and choice instances in `TypeEnvironment`.
//! - validation of nominal bound evidence after a generic instance `TypeId` is interned.
//!
//! This module does NOT own:
//! - generic parameter syntax or scope resolution (lives in `generic_parameters.rs`).
//! - generic base lookup, named/namespaced type resolution, or trait-name rejection
//!   (live in `lookup.rs`).
//! - generic function instantiation (lives in the generic function emission path).

use crate::compiler_frontend::ast::generic_bounds::{
    GenericBoundEvidenceContext, validate_nominal_generic_bound_evidence,
};
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionResult, context::TypeResolutionContext,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::headers::module_symbols::GenericDeclarationKind;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;

/// Resolves a generic struct or choice annotation with concrete type arguments.
///
/// WHAT: interns a canonical generic instance in `TypeEnvironment` from already-resolved argument
///       identities.
/// WHY: generic structs/choices must have concrete `TypeId` identity before HIR lowering.
///
/// Returns `Ok(Some(TypeId))` on successful instantiation, `Ok(None)` when template data is not
/// available (call site should fall back to its unresolved generic representation), or `Err` on
/// failure.
pub(super) fn instantiate_generic_nominal(
    base_path: &PathId,
    kind: &GenericDeclarationKind,
    arguments: &mut Vec<TypeId>,
    instance_name: Option<StringId>,
    span: Option<SourceSpan>,
    context: &mut TypeResolutionContext<'_>,
) -> TypeResolutionResult<Option<TypeId>> {
    match kind {
        GenericDeclarationKind::Struct => {
            let Some(fields_map) = context.resolved_struct_fields_by_path else {
                // Template data unavailable; caller should fall back to its unresolved form.
                return Ok(None);
            };
            if !fields_map.contains_key(base_path) {
                // Template not yet available (e.g. recursive generic type during its own
                // resolution). Fall back so the caller can reject it with a proper diagnostic.
                return Ok(None);
            }

            let type_id = intern_generic_instance_type_id(
                base_path,
                std::mem::take(arguments).into_boxed_slice(),
                context,
            );
            validate_nominal_bound_evidence_for_instantiation(
                type_id,
                instance_name,
                span,
                context,
            )?;
            Ok(Some(type_id))
        }
        GenericDeclarationKind::Choice => {
            let type_id = intern_generic_instance_type_id(
                base_path,
                std::mem::take(arguments).into_boxed_slice(),
                context,
            );
            validate_nominal_bound_evidence_for_instantiation(
                type_id,
                instance_name,
                span,
                context,
            )?;
            Ok(Some(type_id))
        }
        GenericDeclarationKind::Function => unreachable!(
            "generic nominal instantiation reached lookup with a function declaration kind"
        ),
    }
}

/// Intern a canonical `TypeId` for a generic nominal instance.
///
/// WHAT: interns the constructor-plus-resolved-argument identity in `TypeEnvironment`.
/// WHY: struct and choice instantiation both need the same interning step; this helper removes
///      the duplication between the two branches.
///
/// The base path has already been validated as a registered generic nominal by the caller.
fn intern_generic_instance_type_id(
    base_path: &PathId,
    arguments: Box<[TypeId]>,
    context: &mut TypeResolutionContext<'_>,
) -> TypeId {
    let type_environment = &mut *context.type_environment;
    let nominal_id = type_environment
        .nominal_id_for_path(base_path)
        .expect("generic nominal registration must include a nominal identity");

    type_environment.intern_generic_instance(nominal_id, arguments)
}

/// Validate declaration-site trait-bound evidence for a freshly interned nominal instance.
///
/// WHAT: builds the evidence context from the type-resolution context and runs the shared
///       nominal bound validator on the interned instance.
/// WHY: bound evidence must be checked after the instance has a canonical `TypeId` so the
///      validator can inspect `TypeEnvironment` definitions and recursively check arguments.
fn validate_nominal_bound_evidence_for_instantiation(
    type_id: TypeId,
    instance_name: Option<StringId>,
    span: Option<SourceSpan>,
    context: &TypeResolutionContext<'_>,
) -> TypeResolutionResult<()> {
    let evidence_context = GenericBoundEvidenceContext {
        type_environment: context.type_environment,
        trait_environment: context.trait_environment,
        trait_evidence_environment: context.trait_evidence_environment,
        generated_evidence_pairs: None,
        visible_trait_names: context.visible_trait_names,
        visible_source_names: context.visible_source_bindings,
        visible_type_alias_names: context.visible_type_aliases,
        visible_namespace_records: context.visible_namespace_records,
        resolved_type_aliases: context.resolved_type_aliases,
    };

    validate_nominal_generic_bound_evidence(type_id, instance_name, span, &evidence_context)
}
