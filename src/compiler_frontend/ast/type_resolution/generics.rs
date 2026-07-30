//! Lazy nominal generic instance materialization for AST type resolution.
//!
//! WHAT: materializes concrete `TypeId`s for user-declared generic structs and choices once
//!       their base path and resolved type arguments are known, then validates declaration-site
//!       trait-bound evidence for the interned instance.
//! WHY: generic nominal instantiation is a self-contained concern that bridges resolved
//!      diagnostic type arguments to canonical `TypeEnvironment` identity and static bound checks.
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
//! - diagnostic-type-to-`TypeId` conversion helpers (live in `resolve_type.rs`).

use crate::compiler_frontend::ast::generic_bounds::{
    GenericBoundEvidenceContext, validate_nominal_generic_bound_evidence,
};
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionResult, context::TypeResolutionContext,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidGenericInstantiationReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::generic_identity_bridge::{
    GenericInstantiationKey, TypeIdentityKey, data_type_to_type_identity_key,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::headers::module_symbols::{
    GenericDeclarationKind, GenericDeclarationMetadata,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use super::resolve_type::resolve_diagnostic_type_to_type_id;

/// Resolves a generic struct or choice annotation with concrete type arguments.
///
/// WHAT: interns a canonical generic instance in `TypeEnvironment` and returns display
///       spelling for diagnostics and HIR compatibility metadata.
/// WHY: generic structs/choices must have concrete `TypeId` identity before HIR lowering.
///
/// Returns `Ok(Some(DataType))` on successful instantiation, `Ok(None)` when template data
/// is not available (call site should fall back to GenericInstance), or `Err` on failure.
pub(super) fn instantiate_generic_nominal(
    base_path: &InternedPath,
    metadata: &GenericDeclarationMetadata,
    arguments: &[DataType],
    location: &SourceLocation,
    context: &mut TypeResolutionContext<'_>,
) -> TypeResolutionResult<Option<DataType>> {
    let param_count = metadata.parameters.len();
    if arguments.len() != param_count {
        return Err(Box::new(CompilerDiagnostic::invalid_generic_instantiation(
            base_path.name(),
            InvalidGenericInstantiationReason::WrongArgumentCount {
                expected: param_count,
                found: arguments.len(),
            },
            location.to_owned(),
        )));
    }

    // Build argument identity keys for the HIR/diagnostic compatibility bridge.
    // If any argument cannot be keyed (for example, `T` in an unresolved generic
    // function body), the canonical TypeId instance is still interned while the
    // bridge `GenericInstantiationKey` is omitted from display-only DataType data.
    let argument_keys: Option<Vec<TypeIdentityKey>> = arguments
        .iter()
        .map(data_type_to_type_identity_key)
        .collect();
    let instance_key = argument_keys.map(|arguments| GenericInstantiationKey {
        base_path: base_path.to_owned(),
        arguments,
    });

    let instantiated = match metadata.kind {
        GenericDeclarationKind::Struct => {
            let Some(fields_map) = context.resolved_struct_fields_by_path else {
                // Template data unavailable; caller should fall back to GenericInstance.
                return Ok(None);
            };
            if !fields_map.contains_key(base_path) {
                // Template not yet available (e.g. recursive generic type during its own
                // resolution). Fall back to GenericInstance so the caller can reject it
                // with a proper recursive-type diagnostic.
                return Ok(None);
            }

            let type_id = intern_generic_instance_type_id(base_path, arguments, context);
            validate_nominal_bound_evidence_for_instantiation(type_id, location, context)?;

            DataType::Struct {
                nominal_path: base_path.to_owned(),
                type_id,
                const_record: false,
                generic_instance_key: instance_key.to_owned(),
            }
        }
        GenericDeclarationKind::Choice => {
            let type_id = intern_generic_instance_type_id(base_path, arguments, context);
            validate_nominal_bound_evidence_for_instantiation(type_id, location, context)?;

            DataType::Choices {
                nominal_path: base_path.to_owned(),
                type_id,
                generic_instance_key: instance_key.to_owned(),
            }
        }
        _ => {
            // Not a generic struct or choice; fall back to GenericInstance.
            return Ok(None);
        }
    };

    Ok(Some(instantiated))
}

/// Intern a canonical `TypeId` for a generic nominal instance.
///
/// WHAT: resolves each argument to a semantic `TypeId` and interns the constructor-plus-arguments
///       shape in `TypeEnvironment`.
/// WHY: struct and choice instantiation both need the same interning step; this helper removes
///      the duplication between the two branches.
///
/// Returns the builtin `None` type when the base path has no registered nominal identity.
/// Callers already validate the base before reaching this point, so that fallback is a defensive
/// placeholder rather than a user-facing error.
fn intern_generic_instance_type_id(
    base_path: &InternedPath,
    arguments: &[DataType],
    context: &mut TypeResolutionContext<'_>,
) -> TypeId {
    let type_environment = &mut *context.type_environment;
    let Some(nominal_id) = type_environment.nominal_id_for_path(base_path) else {
        return type_environment.builtins().none;
    };

    let arg_type_ids: Box<[TypeId]> = arguments
        .iter()
        .map(|arg| resolve_diagnostic_type_to_type_id(arg, type_environment))
        .collect();

    type_environment.intern_generic_instance(nominal_id, arg_type_ids)
}

/// Validate declaration-site trait-bound evidence for a freshly interned nominal instance.
///
/// WHAT: builds the evidence context from the type-resolution context and runs the shared
///       nominal bound validator on the interned instance.
/// WHY: bound evidence must be checked after the instance has a canonical `TypeId` so the
///      validator can inspect `TypeEnvironment` definitions and recursively check arguments.
fn validate_nominal_bound_evidence_for_instantiation(
    type_id: TypeId,
    location: &SourceLocation,
    context: &TypeResolutionContext<'_>,
) -> TypeResolutionResult<()> {
    let evidence_context = GenericBoundEvidenceContext {
        type_environment: context.type_environment,
        trait_environment: context.trait_environment,
        trait_evidence_environment: context.trait_evidence_environment,
        visible_trait_names: context.visible_trait_names,
        visible_source_names: context.visible_source_bindings,
        visible_type_alias_names: context.visible_type_aliases,
        visible_namespace_records: context.visible_namespace_records,
        resolved_type_aliases: context.resolved_type_aliases,
    };

    validate_nominal_generic_bound_evidence(type_id, location.clone(), &evidence_context)
}
