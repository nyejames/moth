//! Definition-table and frontend type-link validation for HIR.
//!
//! WHAT: builds the validator lookup tables and checks that HIR nominal layouts still point
//! back to canonical frontend type definitions.
//! WHY: later validation families depend on these tables, and backends rely on the
//! frontend-type links for layout identity.

use super::HirValidator;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::hir_side_table::HirLocation;

impl<'a> HirValidator<'a> {
    // -------------------------
    //  Definition Collection
    // -------------------------

    pub(super) fn collect_definition_ids(&mut self) -> Result<(), CompilerError> {
        for region in &self.module.regions {
            let id = region.id();
            if !self.region_ids.insert(id) {
                return Err(self.error_with_hir(format!("Duplicate HIR region id {id:?}"), None));
            }
        }

        for block in &self.module.blocks {
            if !self.block_ids.insert(block.id) {
                return Err(self.error_with_hir(
                    format!("Duplicate HIR block id {:?}", block.id),
                    Some(HirLocation::Block(block.id)),
                ));
            }
            self.block_index_by_id
                .insert(block.id, self.block_index_by_id.len());

            for local in &block.locals {
                if self.local_types.insert(local.id, local.ty).is_some() {
                    return Err(self.error_with_hir(
                        format!("Duplicate HIR local id {:?}", local.id),
                        Some(HirLocation::Block(block.id)),
                    ));
                }
                self.local_block_by_id.insert(local.id, block.id);
            }

            for statement in &block.statements {
                if self
                    .statement_block_by_id
                    .insert(statement.id, block.id)
                    .is_some()
                {
                    return Err(self.error_with_hir(
                        format!("Duplicate HIR statement id {:?}", statement.id),
                        Some(HirLocation::Block(block.id)),
                    ));
                }
            }
        }

        for hir_struct in &self.module.structs {
            if self
                .struct_types
                .insert(hir_struct.id, hir_struct.frontend_type_id)
                .is_some()
            {
                return Err(self.error_with_hir(
                    format!("Duplicate HIR struct id {:?}", hir_struct.id),
                    Some(HirLocation::Struct(hir_struct.id)),
                ));
            }

            for field in &hir_struct.fields {
                if !self.field_ids.insert(field.id) {
                    return Err(self.error_with_hir(
                        format!("Duplicate HIR field id {:?}", field.id),
                        Some(HirLocation::Struct(hir_struct.id)),
                    ));
                }

                self.field_types.insert(field.id, field.ty);
                self.field_owner.insert(field.id, hir_struct.id);
            }
        }

        for function in &self.module.functions {
            if !self.function_ids.insert(function.id) {
                return Err(self.error_with_hir(
                    format!("Duplicate HIR function id {:?}", function.id),
                    Some(HirLocation::Function(function.id)),
                ));
            }
            self.parameter_local_ids
                .extend(function.params.iter().copied());
        }

        Ok(())
    }

    // -------------------------
    //  Type Layout Validation
    // -------------------------

    /// Verify that every HIR struct and choice carries a `frontend_type_id`
    /// that resolves to a real entry in the frontend `TypeEnvironment`.
    ///
    /// WHY: `frontend_type_id` is the canonical link from lowering-local layout
    ///      back to semantic type identity. If it is orphan or zero, backend
    ///      type lookups can silently produce wrong layouts.
    pub(super) fn validate_frontend_type_ids(&self) -> Result<(), CompilerError> {
        for hir_struct in &self.module.structs {
            self.require_type_id(
                hir_struct.frontend_type_id,
                Some(HirLocation::Struct(hir_struct.id)),
            )?;

            for field in &hir_struct.fields {
                self.require_type_id(field.ty, Some(HirLocation::Struct(hir_struct.id)))?;
            }
        }

        for choice in &self.module.choices {
            self.require_type_id(
                choice.frontend_type_id,
                Some(HirLocation::Choice(choice.id)),
            )?;

            for variant in &choice.variants {
                for field in &variant.fields {
                    self.require_type_id(field.ty, Some(HirLocation::Choice(choice.id)))?;
                }
            }
        }

        Ok(())
    }
}
