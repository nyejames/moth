//! HIR type-lowering helpers.
//!
//! WHAT: validates frontend `TypeId`s and manages generic struct/choice registration for HIR.
//! WHY: HIR now carries frontend semantic `TypeId`s directly. This module owns generic
//!      nominal instance registration and defensive type-id validation.
//!
//! Compatibility boundary:
//! `GenericInstantiationKey` is retained here only as a lowering-local side-table key for
//! generic struct/choice layout registration. It is derived from canonical frontend `TypeId`
//! information before HIR and must not become a second semantic type identity system.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceVariantPayloadDefinition, TypeDefinition,
};
use crate::compiler_frontend::datatypes::generic_identity_bridge::generic_instantiation_key_argument_type_ids;
use crate::compiler_frontend::datatypes::ids::TypeId as FrontendTypeId;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::module::{HirChoice, HirChoiceField, HirChoiceVariant};
use crate::compiler_frontend::hir::structs::{HirField, HirStruct};
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

impl<'a> HirBuilder<'a> {
    /// Verifies that a frontend `TypeId` is registered in `TypeEnvironment`.
    ///
    /// WHAT: HIR now uses frontend `TypeId`s directly, so this is a defensive check.
    /// WHY: catches unregistered types that slipped through AST resolution.
    pub(crate) fn lower_type_id(
        &mut self,
        type_id: FrontendTypeId,
        span: &Option<SourceSpan>,
    ) -> Result<TypeId, CompilerError> {
        match self.type_environment.get(type_id) {
            Some(TypeDefinition::GenericParameter(parameter)) => {
                return_hir_transformation_error!(
                    format!(
                        "Unresolved generic parameter TypeId {:?} ({:?}) reached HIR lowering",
                        type_id, parameter.id
                    ),
                    self.hir_error_location(span)
                );
            }
            Some(_) => Ok(type_id),
            None => {
                return_hir_transformation_error!(
                    format!("TypeId {:?} is not registered in TypeEnvironment", type_id),
                    self.hir_error_location(span)
                );
            }
        }
    }

    pub(crate) fn resolve_or_register_generic_struct(
        &mut self,
        key: &crate::compiler_frontend::datatypes::generic_identity_bridge::GenericInstantiationKey,
        nominal_path: &crate::compiler_frontend::symbols::interned_path::InternedPath,
        _type_id: crate::compiler_frontend::datatypes::ids::TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<crate::compiler_frontend::hir::ids::StructId, CompilerError> {
        if let Some(&struct_id) = self.generic_structs_by_key.get(key) {
            return Ok(struct_id);
        }

        // Compute the generic instance TypeId in the frontend TypeEnvironment so that
        // substituted fields are available for backend lowering.
        let instance_type_id = {
            let nominal_id = self
                .type_environment
                .nominal_id_for_path(nominal_path)
                .ok_or_else(|| {
                    crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                        format!(
                            "Base struct '{}' not found in HIR TypeEnvironment",
                            nominal_path.to_string(self.string_table)
                        ),
                    )
                })?;
            let arg_ids = generic_instantiation_key_argument_type_ids(
                key,
                &mut self.type_environment,
            )
            .ok_or_else(|| {
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    format!(
                        "Generic struct '{}' has an argument that is not registered in HIR TypeEnvironment",
                        nominal_path.to_string(self.string_table)
                    ),
                )
            })?;
            self.type_environment
                .intern_generic_instance(nominal_id, arg_ids)
        };

        let struct_id = self.allocate_struct_id();
        let mut hir_fields = Vec::new();

        // Copy only compact field facts before mutating the builder. Keeping a
        // borrowed `TypeEnvironment` view across `self` mutations would conflict
        // with field-id allocation and defensive type validation.
        let field_definitions: Vec<_> = self
            .type_environment
            .fields_for(instance_type_id)
            .map(|fields| {
                fields
                    .iter()
                    .map(|field| (field.name.clone(), field.type_id))
                    .collect()
            })
            .unwrap_or_default();

        for (field_name, field_type_id) in field_definitions {
            let field_type = self.lower_type_id(field_type_id, span)?;
            let field_id = self.allocate_field_id();
            self.fields_by_struct_and_name
                .insert((struct_id, field_name.clone()), field_id);
            self.side_table.bind_field_name(field_id, field_name);
            hir_fields.push(HirField {
                id: field_id,
                ty: field_type,
            });
        }

        let hir_struct = HirStruct {
            id: struct_id,
            frontend_type_id: instance_type_id,
            fields: hir_fields,
        };

        self.generic_structs_by_key
            .insert(key.to_owned(), struct_id);
        self.side_table
            .bind_struct_name(struct_id, nominal_path.to_owned());
        self.side_table
            .bind_generic_struct_instance(struct_id, key.to_owned());
        self.push_struct(hir_struct);

        Ok(struct_id)
    }

    pub(crate) fn resolve_or_register_generic_choice(
        &mut self,
        key: &crate::compiler_frontend::datatypes::generic_identity_bridge::GenericInstantiationKey,
        nominal_path: &crate::compiler_frontend::symbols::interned_path::InternedPath,
        _type_id: crate::compiler_frontend::datatypes::ids::TypeId,
        _span: &Option<SourceSpan>,
    ) -> Result<crate::compiler_frontend::hir::ids::ChoiceId, CompilerError> {
        if let Some(&choice_id) = self.generic_choices_by_key.get(key) {
            return Ok(choice_id);
        }

        // Compute the generic instance TypeId in the frontend TypeEnvironment so that
        // substituted variants are available for backend lowering.
        let instance_type_id = {
            let nominal_id = self
                .type_environment
                .nominal_id_for_path(nominal_path)
                .ok_or_else(|| {
                    crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                        format!(
                            "Base choice '{}' not found in HIR TypeEnvironment",
                            nominal_path.to_string(self.string_table)
                        ),
                    )
                })?;
            let arg_ids = generic_instantiation_key_argument_type_ids(
                key,
                &mut self.type_environment,
            )
            .ok_or_else(|| {
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    format!(
                        "Generic choice '{}' has an argument that is not registered in HIR TypeEnvironment",
                        nominal_path.to_string(self.string_table)
                    ),
                )
            })?;
            self.type_environment
                .intern_generic_instance(nominal_id, arg_ids)
        };

        let choice_id = self.allocate_choice_id();
        let hir_variants = self.lower_choice_variants_for_type_id(instance_type_id, _span)?;

        self.generic_choices_by_key
            .insert(key.to_owned(), choice_id);
        self.side_table
            .bind_choice_name(choice_id, nominal_path.to_owned());
        self.side_table
            .bind_generic_choice_instance(choice_id, key.to_owned());
        let index = choice_id.0 as usize;
        debug_assert!(index == self.module.choices.len());
        self.module.choices.push(HirChoice {
            id: choice_id,
            frontend_type_id: instance_type_id,
            variants: hir_variants,
        });

        Ok(choice_id)
    }

    pub(crate) fn lower_choice_variants_for_type_id(
        &mut self,
        type_id: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<Vec<crate::compiler_frontend::hir::module::HirChoiceVariant>, CompilerError> {
        // Copy compact variant facts before allocating/lowering fields. This keeps
        // the TypeEnvironment as the single metadata owner while avoiding a long
        // immutable borrow across `self` mutations.
        let variant_definitions = {
            let Some(variants) = self.type_environment.variants_for(type_id) else {
                return_hir_transformation_error!(
                    format!("Choice TypeId {:?} has no variant metadata", type_id),
                    self.hir_error_location(span)
                );
            };

            let mut lowered_variants = Vec::with_capacity(variants.len());
            for variant in variants {
                let mut lowered_fields = Vec::new();

                if let ChoiceVariantPayloadDefinition::Record { fields } = &variant.payload {
                    lowered_fields.reserve(fields.len());

                    for field in fields {
                        let Some(field_name) = field.name.name() else {
                            return_hir_transformation_error!(
                                "Choice variant field is missing a name",
                                self.hir_error_location(span)
                            );
                        };
                        lowered_fields.push((field_name, field.type_id));
                    }
                }

                lowered_variants.push((variant.name, lowered_fields));
            }

            lowered_variants
        };

        let mut hir_variants = Vec::with_capacity(variant_definitions.len());
        for (variant_name, variant_fields) in variant_definitions {
            let mut hir_variant_fields = Vec::with_capacity(variant_fields.len());

            for (field_name, field_type_id) in variant_fields {
                let field_type = self.lower_type_id(field_type_id, span)?;
                hir_variant_fields.push(HirChoiceField {
                    name: field_name,
                    ty: field_type,
                });
            }

            hir_variants.push(HirChoiceVariant {
                name: variant_name,
                fields: hir_variant_fields,
            });
        }

        Ok(hir_variants)
    }
}
