//! Semantic resolution of authored type syntax into canonical `TypeId`s.
//!
//! WHAT: resolves retained `ParsedTypeRef` syntax directly against visible declarations,
//!       aliases, generic parameters, and the active `TypeEnvironment`.
//! WHY: semantic AST nodes carry canonical type identity, while authored syntax stays with its
//!      declaration or signature.

use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionResult,
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
    CompilerDiagnostic, GenericApplicationErrorReason, InvalidCollectionTypeReason,
    InvalidMapTypeReason, InvalidTypeAnnotationReason,
};
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::parsed::{ParsedCollectionCapacity, ParsedTypeRef};
use crate::compiler_frontend::declaration_syntax::type_syntax::TypeAnnotationContext;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Resolve a source-authored annotation directly to canonical identity.
///
/// `None` means the syntax is inferred or cannot yet resolve because its declaration has not
/// completed registration. The caller retains the `ParsedTypeRef` and can resolve it again at
/// the correct phase boundary.
pub(crate) fn resolve_parsed_type_annotation(
    source_ref: &ParsedTypeRef,
    span: Option<SourceSpan>,
    context: &mut TypeResolutionContext<'_>,
    string_table: &mut StringTable,
    scope_context: Option<&ScopeContext>,
) -> TypeResolutionResult<ResolvedTypeAnnotation> {
    let type_id = resolve_parsed_type_ref(source_ref, span, context, string_table, scope_context)?;
    Ok(ResolvedTypeAnnotation { type_id })
}

/// Reuse a parser-resolved identity when the later phase has no additional resolution work.
///
/// WHAT: returns a retained `TypeId` only when its parsed syntax needs neither generic-context
///       resolution, generic-bound validation, nor deferred constant-capacity folding.
/// WHY: signature and member parsing already resolves ordinary annotations; resolving the same
///      retained names again during environment registration repeats visible type lookups.
pub(crate) fn reusable_type_id_for_parsed_ref(
    source_ref: &ParsedTypeRef,
    candidate: Option<TypeId>,
    context: &TypeResolutionContext<'_>,
) -> Option<TypeId> {
    let type_id = candidate?;
    if type_id == context.type_environment.builtins().none
        || context.type_environment.get(type_id).is_none()
        || parsed_type_ref_needs_late_resolution(source_ref, context)
    {
        return None;
    }

    Some(type_id)
}

fn parsed_type_ref_needs_late_resolution(
    source_ref: &ParsedTypeRef,
    context: &TypeResolutionContext<'_>,
) -> bool {
    match source_ref {
        ParsedTypeRef::Inferred | ParsedTypeRef::This { .. } => true,
        ParsedTypeRef::Named { name, .. } => context
            .generic_parameters
            .is_some_and(|scope| scope.contains_name(*name)),
        ParsedTypeRef::Qualified { .. } => false,
        ParsedTypeRef::Applied { .. } => true,
        ParsedTypeRef::BuiltinBool { .. }
        | ParsedTypeRef::BuiltinString { .. }
        | ParsedTypeRef::BuiltinChar { .. }
        | ParsedTypeRef::BuiltinFixedScalar { .. }
        | ParsedTypeRef::BuiltinNumber { .. } => false,
        ParsedTypeRef::Collection {
            element,
            fixed_capacity,
            ..
        } => {
            matches!(
                fixed_capacity,
                Some(ParsedCollectionCapacity::BareConstant { .. })
            ) || parsed_type_ref_needs_late_resolution(element, context)
        }
        ParsedTypeRef::Map { key, value, .. } => {
            parsed_type_ref_needs_late_resolution(key, context)
                || parsed_type_ref_needs_late_resolution(value, context)
        }
        ParsedTypeRef::Optional { inner, .. } => {
            parsed_type_ref_needs_late_resolution(inner, context)
        }
    }
}

fn parsed_type_ref_span(source_ref: &ParsedTypeRef) -> Option<SourceSpan> {
    match source_ref {
        ParsedTypeRef::Inferred => None,
        ParsedTypeRef::Named { span, .. }
        | ParsedTypeRef::Qualified { span, .. }
        | ParsedTypeRef::Applied { span, .. }
        | ParsedTypeRef::BuiltinBool { span, .. }
        | ParsedTypeRef::BuiltinString { span, .. }
        | ParsedTypeRef::BuiltinChar { span, .. }
        | ParsedTypeRef::BuiltinFixedScalar { span, .. }
        | ParsedTypeRef::BuiltinNumber { span, .. }
        | ParsedTypeRef::This { span, .. }
        | ParsedTypeRef::Collection { span, .. }
        | ParsedTypeRef::Map { span, .. }
        | ParsedTypeRef::Optional { span, .. } => *span,
    }
}

fn resolve_parsed_type_ref(
    source_ref: &ParsedTypeRef,
    span: Option<SourceSpan>,
    context: &mut TypeResolutionContext<'_>,
    string_table: &mut StringTable,
    scope_context: Option<&ScopeContext>,
) -> TypeResolutionResult<Option<TypeId>> {
    increment_ast_counter(AstCounter::TypeResolutionCalls);
    let span = parsed_type_ref_span(source_ref).or(span);
    match source_ref {
        ParsedTypeRef::Inferred | ParsedTypeRef::This { .. } => Ok(None),
        ParsedTypeRef::BuiltinBool { .. } => Ok(Some(context.type_environment.builtins().bool)),
        ParsedTypeRef::BuiltinString { .. } => Ok(Some(context.type_environment.builtins().string)),
        ParsedTypeRef::BuiltinChar { .. } => Ok(Some(context.type_environment.builtins().char)),
        ParsedTypeRef::BuiltinFixedScalar { scalar, .. } => {
            Ok(Some(builtin_type_ids::fixed_scalar(*scalar)))
        }
        ParsedTypeRef::BuiltinNumber { scale, .. } => {
            Ok(Some(context.type_environment.intern_number(*scale)))
        }
        ParsedTypeRef::Named { name, .. } => {
            resolve_named_type_from_context(*name, span, context, string_table)
        }
        ParsedTypeRef::Qualified { path, .. } => {
            resolve_namespaced_type_from_context(path, span, context)
        }
        ParsedTypeRef::Applied {
            base, arguments, ..
        } => resolve_applied_type(base, arguments, span, context, string_table, scope_context),
        ParsedTypeRef::Collection {
            element,
            fixed_capacity,
            ..
        } => {
            // Resolve the element before folding capacity, matching the source diagnostic order.
            let element_type_id =
                resolve_parsed_type_ref(element, span, context, string_table, scope_context)?;
            let fixed_capacity = match fixed_capacity {
                Some(capacity) => match fold_collection_capacity(
                    capacity,
                    scope_context,
                    context.type_environment,
                    context.const_values,
                ) {
                    Ok(value) => Some(value),
                    Err(diagnostic)
                        if scope_context.is_none()
                            && matches!(
                                &diagnostic.as_diagnostic().payload,
                                crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidCollectionType {
                                    reason: InvalidCollectionTypeReason::CapacityNotConstant,
                                    ..
                                }
                            ) =>
                    {
                        // Header-only generic bounds can be resolved before a value scope exists.
                        // Preserve their old unsized-collection fallback and retained syntax.
                        None
                    }
                    Err(diagnostic) => return Err(diagnostic.into_diagnostic()),
                },
                None => None,
            };

            let Some(element_type_id) = element_type_id else {
                return Ok(None);
            };
            Ok(Some(
                context
                    .type_environment
                    .intern_collection(element_type_id, fixed_capacity),
            ))
        }
        ParsedTypeRef::Map { key, value, .. } => {
            // Validate inline nesting before resolving either member.
            let nesting_depth = map_nesting_depth(source_ref);
            if nesting_depth > 2 {
                return Err(CompilerDiagnostic::invalid_map_type(
                    InvalidMapTypeReason::ExcessiveInlineNesting {
                        depth: nesting_depth,
                    },
                    span,
                ));
            }

            let key_type_id =
                resolve_parsed_type_ref(key, span, context, string_table, scope_context)?;
            let value_type_id =
                resolve_parsed_type_ref(value, span, context, string_table, scope_context)?;
            let (Some(key_type_id), Some(value_type_id)) = (key_type_id, value_type_id) else {
                return Ok(None);
            };

            validate_map_key_type(key_type_id, context.type_environment, span)?;
            Ok(Some(
                context
                    .type_environment
                    .intern_map(key_type_id, value_type_id),
            ))
        }
        ParsedTypeRef::Optional { inner, .. } => {
            let Some(inner_type_id) =
                resolve_parsed_type_ref(inner, span, context, string_table, scope_context)?
            else {
                return Ok(None);
            };

            if context
                .type_environment
                .option_inner_type(inner_type_id)
                .is_some()
            {
                return Err(CompilerDiagnostic::invalid_type_annotation(
                    TypeAnnotationContext::DeclarationTarget,
                    InvalidTypeAnnotationReason::NestedOptional,
                    span,
                ));
            }

            Ok(Some(context.type_environment.intern_option(inner_type_id)))
        }
    }
}

fn resolve_applied_type(
    base: &ParsedTypeRef,
    arguments: &[ParsedTypeRef],
    span: Option<SourceSpan>,
    context: &mut TypeResolutionContext<'_>,
    string_table: &mut StringTable,
    scope_context: Option<&ScopeContext>,
) -> TypeResolutionResult<Option<TypeId>> {
    let ParsedTypeRef::Named { name, .. } = base else {
        return Err(CompilerDiagnostic::invalid_generic_application(
            GenericApplicationErrorReason::OnNonNamedType,
            span,
        ));
    };

    let base_path = resolve_generic_base_type(*name, arguments.len(), span, context, string_table)?;

    let mut argument_type_ids = Vec::with_capacity(arguments.len());
    for argument in arguments {
        let Some(type_id) =
            resolve_parsed_type_ref(argument, span, context, string_table, scope_context)?
        else {
            return Ok(None);
        };
        argument_type_ids.push(type_id);
    }

    if let Some(kind) = context
        .generic_declarations_by_path
        .and_then(|declarations| declarations.get(&base_path))
        && let Some(type_id) = instantiate_generic_nominal(
            &base_path,
            kind,
            &mut argument_type_ids,
            Some(*name),
            span,
            context,
        )?
    {
        return Ok(Some(type_id));
    }

    let Some(nominal_id) = context.type_environment.nominal_id_for_path(&base_path) else {
        return Ok(None);
    };
    Ok(Some(context.type_environment.intern_generic_instance(
        nominal_id,
        argument_type_ids.into_boxed_slice(),
    )))
}
