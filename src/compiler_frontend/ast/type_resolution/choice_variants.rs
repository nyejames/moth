//! Choice variant payload type resolution for AST type resolution.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionContext, TypeResolutionResult, resolve_parsed_type_annotation,
    reusable_type_id_for_parsed_ref,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTypeAnnotationReason, TypeAnnotationContext,
};
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::symbols::string_interning::StringTable;

// ----------------------------------
//  Choice variant payload resolution
// ----------------------------------

/// Resolve choice payload `TypeId`s from retained parsed annotations.
/// Authored syntax stays on each field for diagnostics and later re-resolution.
pub(crate) fn resolve_choice_variant_payload_types(
    variants: &[ChoiceVariant],
    type_resolution_context: &mut TypeResolutionContext<'_>,
    string_table: &mut StringTable,
    scope_context: Option<&ScopeContext>,
) -> TypeResolutionResult<Vec<ChoiceVariant>> {
    let mut resolved_variants = Vec::with_capacity(variants.len());

    for variant in variants {
        let payload = match &variant.payload {
            ChoiceVariantPayload::Unit => ChoiceVariantPayload::Unit,

            ChoiceVariantPayload::Record { fields } => {
                let mut resolved_fields = Vec::with_capacity(fields.len());

                for field in fields {
                    let mut resolved_field = field.to_owned();

                    resolved_field.type_id = if let Some(type_syntax) = &field.type_syntax {
                        let type_id = if let Some(type_id) = reusable_type_id_for_parsed_ref(
                            type_syntax,
                            Some(field.type_id),
                            type_resolution_context,
                        ) {
                            Some(type_id)
                        } else {
                            resolve_parsed_type_annotation(
                                type_syntax,
                                field.binding_span,
                                type_resolution_context,
                                string_table,
                                scope_context,
                            )?
                            .type_id
                        };
                        type_id
                            .filter(|type_id| {
                                *type_id != type_resolution_context.type_environment.builtins().none
                            })
                            .ok_or_else(|| {
                                CompilerDiagnostic::invalid_type_annotation(
                                    TypeAnnotationContext::DeclarationTarget,
                                    InvalidTypeAnnotationReason::NoneNotAllowed,
                                    field.binding_span,
                                )
                            })?
                    } else if field.type_id
                        != type_resolution_context.type_environment.builtins().none
                        && type_resolution_context
                            .type_environment
                            .get(field.type_id)
                            .is_some()
                    {
                        field.type_id
                    } else {
                        return Err(CompilerDiagnostic::invalid_type_annotation(
                            TypeAnnotationContext::DeclarationTarget,
                            InvalidTypeAnnotationReason::NoneNotAllowed,
                            field.binding_span,
                        ));
                    };

                    resolved_fields.push(resolved_field);
                }

                ChoiceVariantPayload::Record {
                    fields: resolved_fields,
                }
            }
        };

        resolved_variants.push(ChoiceVariant {
            id: variant.id,
            payload,
            span: variant.span,
        });
    }

    Ok(resolved_variants)
}
