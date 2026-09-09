//! Choice variant payload type resolution for AST type resolution.

use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionContext, TypeResolutionResult, resolve_diagnostic_type_to_type_id_checked,
};
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use super::resolve_named_signature_type;

// ----------------------------------
//  Choice variant payload resolution
// ----------------------------------

/// Resolve choice payload field types, replacing `NamedType` placeholders in record variants and
/// assigning checked semantic `TypeId`s to payload fields.
pub(crate) fn resolve_choice_variant_payload_types(
    variants: &[ChoiceVariant],
    type_resolution_context: &mut TypeResolutionContext<'_>,
    string_table: &StringTable,
) -> TypeResolutionResult<Vec<ChoiceVariant>> {
    let mut resolved_variants = Vec::with_capacity(variants.len());

    for variant in variants {
        let payload = match &variant.payload {
            ChoiceVariantPayload::Unit => ChoiceVariantPayload::Unit,

            ChoiceVariantPayload::Record { fields } => {
                let mut resolved_fields = Vec::with_capacity(fields.len());

                for field in fields {
                    let mut resolved_field = field.to_owned();

                    resolved_field.value.diagnostic_type = resolve_named_signature_type(
                        &field.value.diagnostic_type,
                        field.value.span,
                        type_resolution_context,
                        string_table,
                    )?;
                    resolved_field.value.type_id = resolve_diagnostic_type_to_type_id_checked(
                        &resolved_field.value.diagnostic_type,
                        type_resolution_context.type_environment,
                        resolved_field.value.span,
                    )?;

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
