//! Semantic type resolution for AST construction.
//!
//! WHAT: resolves parsed type syntax into canonical `TypeId`-based semantic identity using
//! module-visible declarations, type aliases, generic parameters, and external symbols.
//! WHY: semantic type resolution is an AST concern, not a syntax-parsing concern. Moving it here
//!      clarifies the boundary between shared declaration-shell parsing and AST-owned lowering.
//!
//! This module owns:
//! - recursive `resolve_type` from `DataType` parse placeholders to resolved `DataType`
//! - `resolve_parsed_type_annotation` which bridges `ParsedTypeRef` → `DataType` → `TypeId`
//! - checked and optional diagnostic-type-to-`TypeId` conversion helpers
//!
//! This module does NOT own:
//! - `TypeResolutionContext`, its inputs, or `ResolvedTypeAnnotation` (live in `context.rs`)
//! - type alias lookup and alias-target re-resolution (live in `aliases.rs`)
//! - source-visible type-name lookup and trait-name rejection (live in `lookup.rs`)
//! - generic nominal instantiation and bound-evidence checks (live in `generics.rs`)
//! - token-to-parsed-ref parsing (lives in `declaration_syntax::type_syntax`)
//! - parsed-ref walkers or dependency extraction (lives in `declaration_syntax::type_syntax`)
//! - `parsed_ref_to_data_type` syntax-to-diagnostic spelling (lives in `declaration_syntax::type_syntax`)

use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionResult, aliases,
    collections::fold_collection_capacity,
    context::{ResolvedTypeAnnotation, TypeResolutionContext},
    generics::instantiate_generic_nominal,
    lookup::{
        resolve_generic_base_type, resolve_named_type_from_context,
        resolve_namespaced_type_from_context,
    },
    maps::{map_nesting_depth, validate_map_key_type},
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidCollectionTypeReason, InvalidMapTypeReason,
    InvalidTypeAnnotationReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_identity_bridge::{
    BuiltinGenericType, GenericBaseType, generic_instantiation_key_argument_type_ids,
};
use crate::compiler_frontend::datatypes::ids::{FunctionTypeKey, TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::type_syntax::{
    TypeAnnotationContext, parsed_ref_to_data_type,
};
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{SourceLocation, TokenKind};

/// Resolve a parsed type annotation through the parsed-ref-aware path.
///
/// WHAT: folds fixed-collection capacity syntax, re-resolves type aliases from their
///       stored `ParsedTypeRef`, and produces canonical `TypeId` identity.
/// WHY: this is the semantic entry point for all type annotations that start as
///      `ParsedTypeRef`; it must not hide capacity folding inside `parsed_ref_to_data_type`.
pub(crate) fn resolve_parsed_type_annotation(
    source_ref: ParsedTypeRef,
    location: &SourceLocation,
    context: &mut TypeResolutionContext<'_>,
    string_table: &mut StringTable,
    scope_context: Option<&ScopeContext>,
) -> TypeResolutionResult<ResolvedTypeAnnotation> {
    match &source_ref {
        ParsedTypeRef::Collection {
            element,
            fixed_capacity,
            location: collection_location,
        } => {
            let element_annotation = resolve_parsed_type_annotation(
                *element.clone(),
                collection_location,
                context,
                string_table,
                scope_context,
            )?;
            let element_id = element_annotation.type_id.unwrap_or(builtin_type_ids::NONE);

            let folded_capacity = match fixed_capacity {
                Some(capacity) => {
                    match fold_collection_capacity(
                        capacity,
                        scope_context,
                        context.type_environment,
                    ) {
                        Ok(value) => Some(value),
                        Err(diagnostic) => {
                            // Only fall back to the diagnostic path when a bare constant
                            // cannot be resolved because no scope context is available.
                            // Literal invalid values (zero, overflow) must still be rejected.
                            let is_non_constant_because_no_scope = scope_context.is_none()
                                && matches!(
                                    &diagnostic.as_diagnostic().payload,
                                    crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidCollectionType {
                                        reason: InvalidCollectionTypeReason::CapacityNotConstant,
                                        ..
                                    }
                                );
                            if is_non_constant_because_no_scope {
                                return fallback_parsed_ref_to_data_type(
                                    source_ref,
                                    location,
                                    context,
                                    string_table,
                                );
                            }
                            return Err(diagnostic.into_boxed());
                        }
                    }
                }
                None => None,
            };

            let type_id = context
                .type_environment
                .intern_collection(element_id, folded_capacity);
            let diagnostic_type = DataType::GenericInstance {
                base: GenericBaseType::Builtin(BuiltinGenericType::Collection {
                    fixed_capacity: folded_capacity,
                }),
                arguments: vec![element_annotation.diagnostic_type],
            };
            return Ok(ResolvedTypeAnnotation {
                source_ref,
                diagnostic_type,
                type_id: Some(type_id),
            });
        }

        ParsedTypeRef::Map {
            key,
            value,
            location: map_location,
        } => {
            // Validate inline nesting depth before resolution.
            let nesting_depth = map_nesting_depth(&source_ref);
            if nesting_depth > 2 {
                return Err(Box::new(CompilerDiagnostic::invalid_map_type(
                    InvalidMapTypeReason::ExcessiveInlineNesting {
                        depth: nesting_depth,
                    },
                    map_location.clone(),
                )));
            }

            let key_annotation = resolve_parsed_type_annotation(
                *key.clone(),
                map_location,
                context,
                string_table,
                scope_context,
            )?;
            let value_annotation = resolve_parsed_type_annotation(
                *value.clone(),
                map_location,
                context,
                string_table,
                scope_context,
            )?;

            let key_id = key_annotation.type_id.unwrap_or(builtin_type_ids::NONE);
            let value_id = value_annotation.type_id.unwrap_or(builtin_type_ids::NONE);

            // Enforce the scalar-key policy for first-class ordered maps.
            validate_map_key_type(key_id, context.type_environment, map_location)?;

            let type_id = context.type_environment.intern_map(key_id, value_id);
            let diagnostic_type = DataType::GenericInstance {
                base: GenericBaseType::Builtin(BuiltinGenericType::Map),
                arguments: vec![
                    key_annotation.diagnostic_type,
                    value_annotation.diagnostic_type,
                ],
            };
            return Ok(ResolvedTypeAnnotation {
                source_ref,
                diagnostic_type,
                type_id: Some(type_id),
            });
        }

        ParsedTypeRef::Named { name, .. } => {
            if let Some((alias_path, annotation)) =
                aliases::visible_type_alias_annotation(*name, context)
            {
                return aliases::resolve_alias_annotation(
                    alias_path,
                    annotation,
                    location,
                    context,
                    string_table,
                    scope_context,
                );
            }
        }

        ParsedTypeRef::Qualified { path, .. } if path.len() == 2 => {
            // Source type aliases are visible only through shallow namespace records,
            // so a qualified alias reference is always exactly two segments:
            // `namespace.Alias`. Longer paths cannot name a source alias.
            let namespace = path[0];
            let name = path[1];
            if let Some((alias_path, annotation)) =
                aliases::visible_namespaced_type_alias_annotation(namespace, name, context)
            {
                return aliases::resolve_alias_annotation(
                    alias_path,
                    annotation,
                    location,
                    context,
                    string_table,
                    scope_context,
                );
            }
        }

        _ => {}
    }

    fallback_parsed_ref_to_data_type(source_ref, location, context, string_table)
}

/// Fallback path for parsed type annotations that do not need custom handling.
///
/// WHAT: converts `ParsedTypeRef` to diagnostic `DataType` and resolves it through
///       `resolve_type`, preserving the existing behavior for non-collection types.
fn fallback_parsed_ref_to_data_type(
    source_ref: ParsedTypeRef,
    location: &SourceLocation,
    context: &mut TypeResolutionContext<'_>,
    string_table: &StringTable,
) -> TypeResolutionResult<ResolvedTypeAnnotation> {
    let diagnostic_type = parsed_ref_to_data_type(&source_ref);
    let resolved_diagnostic_type = resolve_type(&diagnostic_type, location, context, string_table)?;

    let type_id = if matches!(resolved_diagnostic_type, DataType::Inferred) {
        None
    } else {
        Some(resolve_diagnostic_type_to_type_id_checked(
            &resolved_diagnostic_type,
            context.type_environment,
            location,
        )?)
    };

    Ok(ResolvedTypeAnnotation {
        source_ref,
        diagnostic_type: resolved_diagnostic_type,
        type_id,
    })
}

// ------------------------------------
//  Diagnostic / parse-only DataType helpers
// ------------------------------------

/// Resolve a diagnostic `DataType` spelling into a canonical `TypeId`.
///
/// WHAT: thin wrapper around `resolve_diagnostic_type_to_type_id_opt` that maps `None`
///       to the builtin `None` type so callers do not need to handle the option at every site.
/// WHY: many AST construction paths need a valid `TypeId` for every parsed annotation,
///      even when the nominal type has not yet been registered.
///
/// Prefer `resolve_diagnostic_type_to_type_id_checked` or `resolve_diagnostic_type_to_type_id_opt`
/// when the caller must distinguish unresolved types from the actual `None` type.
pub(crate) fn resolve_diagnostic_type_to_type_id(
    data_type: &DataType,
    type_environment: &mut TypeEnvironment,
) -> TypeId {
    resolve_diagnostic_type_to_type_id_opt(data_type, type_environment)
        .unwrap_or_else(|| type_environment.builtins().none)
}

/// Resolve a fully checked production type spelling into canonical `TypeId` identity.
///
/// WHAT: rejects unresolved parse placeholders instead of mapping them to the builtin `None`
/// type.
/// WHY: executable AST/HIR-bound paths must not silently turn unresolved annotations into
/// valid semantic types.
pub(crate) fn resolve_diagnostic_type_to_type_id_checked(
    data_type: &DataType,
    type_environment: &mut TypeEnvironment,
    location: &SourceLocation,
) -> TypeResolutionResult<TypeId> {
    resolve_diagnostic_type_to_type_id_opt(data_type, type_environment)
        .ok_or_else(|| Box::new(unresolved_type_id_diagnostic(data_type, location)))
}

fn unresolved_type_id_diagnostic(
    data_type: &DataType,
    location: &SourceLocation,
) -> CompilerDiagnostic {
    match data_type {
        DataType::NamedType(name) => {
            CompilerDiagnostic::unknown_type_name(*name, location.to_owned())
        }
        DataType::NamespacedType { path } => {
            if let Some(name) = path.last().copied() {
                CompilerDiagnostic::unknown_type_name(name, location.to_owned())
            } else {
                CompilerDiagnostic::invalid_type_annotation(
                    TypeAnnotationContext::DeclarationTarget,
                    InvalidTypeAnnotationReason::ExpectedTypeAnnotation {
                        found: TokenKind::Eof,
                    },
                    location.to_owned(),
                )
            }
        }
        DataType::GenericInstance {
            base: GenericBaseType::Named(name),
            ..
        } => CompilerDiagnostic::unknown_type_name(*name, location.to_owned()),
        _ => CompilerDiagnostic::invalid_type_annotation(
            TypeAnnotationContext::DeclarationTarget,
            InvalidTypeAnnotationReason::ExpectedTypeAnnotation {
                found: TokenKind::Eof,
            },
            location.to_owned(),
        ),
    }
}

/// Same as `resolve_diagnostic_type_to_type_id`, but returns `None` when the type cannot be
/// meaningfully resolved in the current environment (e.g. unregistered nominal
/// paths or generic instances).
///
/// WHAT: lets callers distinguish "the actual `None` type" from "fallback due
/// to unregistered type".
/// WHY: compatibility checks need to fall back to `TypeIdentityKey` comparison
/// when nominal types have not yet been registered in `TypeEnvironment`.
pub(crate) fn resolve_diagnostic_type_to_type_id_opt(
    data_type: &DataType,
    type_environment: &mut TypeEnvironment,
) -> Option<TypeId> {
    match data_type {
        DataType::Bool => Some(type_environment.builtins().bool),
        DataType::Int => Some(type_environment.builtins().int),
        DataType::Float => Some(type_environment.builtins().float),
        // Decimal is intentionally inactive in the Alpha surface. The reverse lookup
        // is preserved only for diagnostic round-tripping of the inactive builtin.
        DataType::Decimal => Some(type_environment.builtins().decimal),
        DataType::StringSlice => Some(type_environment.builtins().string),
        DataType::Char => Some(type_environment.builtins().char),
        DataType::Range => Some(type_environment.builtins().range),
        DataType::None => Some(type_environment.builtins().none),
        DataType::Template => Some(type_environment.builtins().string),
        DataType::True | DataType::False => Some(type_environment.builtins().bool),
        DataType::Option(inner) => {
            let inner_id = resolve_diagnostic_type_to_type_id_opt(inner, type_environment)?;
            Some(type_environment.intern_option(inner_id))
        }
        DataType::FallibleCarrier { success, error } => {
            let success_id = resolve_diagnostic_type_to_type_id_opt(success, type_environment)?;
            let error_id = resolve_diagnostic_type_to_type_id_opt(error, type_environment)?;
            Some(type_environment.intern_fallible_carrier(success_id, error_id))
        }
        DataType::Struct {
            type_id,
            nominal_path,
            generic_instance_key,
            ..
        }
        | DataType::Choices {
            type_id,
            nominal_path,
            generic_instance_key,
            ..
        } => {
            if *type_id != builtin_type_ids::NONE && generic_instance_key.is_none() {
                Some(*type_id)
            } else if let Some(key) = generic_instance_key {
                let nominal_id = type_environment.nominal_id_for_path(nominal_path)?;
                let arg_ids = generic_instantiation_key_argument_type_ids(key, type_environment)?;
                Some(type_environment.intern_generic_instance(nominal_id, arg_ids))
            } else if *type_id != builtin_type_ids::NONE {
                Some(*type_id)
            } else {
                let nominal_id = type_environment.nominal_id_for_path(nominal_path)?;
                type_environment.type_id_for_nominal_id(nominal_id)
            }
        }
        DataType::TypeParameter {
            canonical_id: Some(canonical_id),
            name,
            ..
        } => Some(type_environment.intern_generic_parameter(*canonical_id, *name)),
        DataType::TypeParameter {
            canonical_id: None, ..
        } => None,
        DataType::GenericInstance { base, arguments } => match base {
            GenericBaseType::Builtin(BuiltinGenericType::Collection { fixed_capacity }) => {
                let element_id =
                    resolve_diagnostic_type_to_type_id_opt(&arguments[0], type_environment)?;
                Some(type_environment.intern_collection(element_id, *fixed_capacity))
            }
            GenericBaseType::Builtin(BuiltinGenericType::Map) => {
                let key_id =
                    resolve_diagnostic_type_to_type_id_opt(&arguments[0], type_environment)?;
                let value_id =
                    resolve_diagnostic_type_to_type_id_opt(&arguments[1], type_environment)?;

                // Map key validation is owned by parsed annotation resolution before this
                // diagnostic-spelling bridge interns the canonical map shape. Callers that
                // accept source-authored map annotations must validate with `validate_map_key_type`.
                Some(type_environment.intern_map(key_id, value_id))
            }
            GenericBaseType::ResolvedNominal(path) => {
                let nominal_id = type_environment.nominal_id_for_path(path)?;
                let arg_ids = arguments
                    .iter()
                    .map(|arg| resolve_diagnostic_type_to_type_id_opt(arg, type_environment))
                    .collect::<Option<Vec<_>>>()?
                    .into_boxed_slice();
                Some(type_environment.intern_generic_instance(nominal_id, arg_ids))
            }
            _ => None,
        },
        DataType::Function(_, signature) => {
            let param_ids: Box<[TypeId]> = signature
                .parameters
                .iter()
                .filter_map(|p| {
                    resolve_diagnostic_type_to_type_id_opt(
                        &p.value.diagnostic_type,
                        type_environment,
                    )
                })
                .collect();
            let return_ids: Box<[TypeId]> = signature
                .returns
                .iter()
                .filter_map(|r| resolve_diagnostic_type_to_type_id_opt(&r.value, type_environment))
                .collect();
            Some(type_environment.intern_function(FunctionTypeKey {
                parameters: param_ids,
                returns: return_ids,
                error_return: None,
            }))
        }
        DataType::Returns(values) => {
            returns_diagnostic_type_to_type_id_opt(values, type_environment)
        }
        DataType::External { type_id } => Some(type_environment.intern_external(*type_id)),
        DataType::Path(_) => Some(type_environment.builtins().string),
        DataType::Parameters(_)
        | DataType::Inferred
        | DataType::NamedType(_)
        | DataType::NamespacedType { .. } => None,
    }
}

pub(crate) fn returns_diagnostic_type_to_type_id_opt(
    values: &[DataType],
    type_environment: &mut TypeEnvironment,
) -> Option<TypeId> {
    match values {
        [] => Some(type_environment.builtins().none),
        [single] => resolve_diagnostic_type_to_type_id_opt(single, type_environment),
        multiple => {
            let field_ids = multiple
                .iter()
                .map(|value| resolve_diagnostic_type_to_type_id_opt(value, type_environment))
                .collect::<Option<Vec<_>>>()?;
            Some(type_environment.intern_tuple(field_ids))
        }
    }
}

// ------------------------------------
//  Type resolution
// ------------------------------------

pub(crate) fn resolve_type(
    data_type: &DataType,
    location: &SourceLocation,
    context: &mut TypeResolutionContext<'_>,
    string_table: &StringTable,
) -> TypeResolutionResult<DataType> {
    increment_ast_counter(AstCounter::TypeResolutionCalls);

    match data_type {
        DataType::NamedType(type_name) => {
            resolve_named_type_from_context(*type_name, location, context, string_table)
        }

        DataType::TypeParameter { .. } => Ok(data_type.to_owned()),
        DataType::GenericInstance { base, arguments } => {
            let resolved_base =
                resolve_generic_base_type(base, arguments, location, context, string_table)?;
            let mut resolved_arguments = Vec::with_capacity(arguments.len());
            for argument in arguments {
                resolved_arguments.push(resolve_type(argument, location, context, string_table)?);
            }

            if matches!(
                resolved_base,
                GenericBaseType::Builtin(BuiltinGenericType::Map)
            ) && let Some(key_type) = resolved_arguments.first()
            {
                let key_id = resolve_diagnostic_type_to_type_id_checked(
                    key_type,
                    context.type_environment,
                    location,
                )?;
                validate_map_key_type(key_id, context.type_environment, location)?;
            }

            // Attempt lazy instantiation for user-declared generic structs/choices.
            if let GenericBaseType::ResolvedNominal(base_path) = &resolved_base
                && let Some(metadata) = context
                    .generic_declarations_by_path
                    .and_then(|decls| decls.get(base_path))
                && let Some(instantiated) = instantiate_generic_nominal(
                    base_path,
                    metadata,
                    &resolved_arguments,
                    location,
                    context,
                )?
            {
                return Ok(instantiated);
            }

            Ok(DataType::GenericInstance {
                base: resolved_base,
                arguments: resolved_arguments,
            })
        }
        DataType::Option(inner) => {
            let resolved_inner = resolve_type(inner, location, context, string_table)?;
            reject_nested_option_type(&resolved_inner, location)?;

            Ok(DataType::Option(Box::new(resolved_inner)))
        }
        DataType::Returns(values) => {
            let mut resolved_values = Vec::with_capacity(values.len());
            for value in values {
                resolved_values.push(resolve_type(value, location, context, string_table)?);
            }
            Ok(DataType::Returns(resolved_values))
        }
        DataType::FallibleCarrier { success, error } => Ok(DataType::fallible_carrier(
            resolve_type(success, location, context, string_table)?,
            resolve_type(error, location, context, string_table)?,
        )),
        DataType::Function(receiver, signature) => {
            let resolved_receiver = receiver
                .as_ref()
                .as_ref()
                .map(|receiver_key| receiver_key.to_owned());

            let mut resolved_signature = signature.to_owned();
            for parameter in &mut resolved_signature.parameters {
                parameter.value.diagnostic_type = resolve_type(
                    &parameter.value.diagnostic_type,
                    &parameter.value.location,
                    context,
                    string_table,
                )?;
            }

            for return_slot in &mut resolved_signature.returns {
                return_slot.value =
                    resolve_type(&return_slot.value, location, context, string_table)?;
            }

            Ok(DataType::Function(
                Box::new(resolved_receiver),
                resolved_signature,
            ))
        }
        DataType::NamespacedType { path } => {
            resolve_namespaced_type_from_context(path, location, context)
        }

        DataType::Struct { .. } | DataType::Choices { .. } => {
            // Struct and choice types no longer carry field/variant payloads in DataType.
            // They are already fully resolved when created; no NamedType placeholders remain.
            Ok(data_type.to_owned())
        }
        DataType::Parameters(parameters) => {
            let mut resolved_parameters = Vec::with_capacity(parameters.len());
            for parameter in parameters {
                let mut resolved_parameter = parameter.to_owned();
                resolved_parameter.value.diagnostic_type = resolve_type(
                    &parameter.value.diagnostic_type,
                    &parameter.value.location,
                    context,
                    string_table,
                )?;
                resolved_parameters.push(resolved_parameter);
            }

            Ok(DataType::Parameters(resolved_parameters))
        }
        _ => Ok(data_type.to_owned()),
    }
}

fn reject_nested_option_type(
    resolved_inner: &DataType,
    location: &SourceLocation,
) -> TypeResolutionResult<()> {
    if matches!(resolved_inner, DataType::Option(_)) {
        return Err(Box::new(CompilerDiagnostic::invalid_type_annotation(
            TypeAnnotationContext::DeclarationTarget,
            InvalidTypeAnnotationReason::NestedOptional,
            location.to_owned(),
        )));
    }

    Ok(())
}
