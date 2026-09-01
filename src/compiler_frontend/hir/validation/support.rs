//! Shared lookup and error helpers for HIR validation.
//!
//! WHAT: keeps ID resolution and source-location fallback in one place for the validation family
//! modules.
//! WHY: validation should report consistent `HirTransformation` infrastructure errors anchored
//! through the HIR side table whenever possible.

use super::HirValidator;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType, SourceLocation};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::hir_side_table::HirLocation;
use crate::compiler_frontend::hir::ids::{BlockId, FieldId, LocalId, RegionId, StructId};

use rustc_hash::FxHashSet;

impl<'a> HirValidator<'a> {
    // -------------------------
    //  Resolution Helpers
    // -------------------------

    pub(super) fn require_block_id(
        &self,
        block_id: BlockId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        if self.block_ids.contains(&block_id) {
            return Ok(());
        }

        Err(self.error_with_hir(format!("Unknown HIR block id {block_id:?}"), anchor))
    }

    pub(super) fn require_struct_id(
        &self,
        struct_id: StructId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        if self.struct_ids.contains(&struct_id) {
            return Ok(());
        }

        Err(self.error_with_hir(format!("Unknown HIR struct id {struct_id:?}"), anchor))
    }

    pub(super) fn require_field_id(
        &self,
        field_id: FieldId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        if self.field_ids.contains(&field_id) {
            return Ok(());
        }

        Err(self.error_with_hir(format!("Unknown HIR field id {field_id:?}"), anchor))
    }

    pub(super) fn require_local_id(
        &self,
        local_id: LocalId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        if self.local_types.contains_key(&local_id) {
            return Ok(());
        }

        Err(self.error_with_hir(format!("Unknown HIR local id {local_id:?}"), anchor))
    }

    pub(super) fn require_region_id(
        &self,
        region_id: RegionId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        if self.region_ids.contains(&region_id) {
            return Ok(());
        }

        Err(self.error_with_hir(format!("Unknown HIR region id {region_id:?}"), anchor))
    }

    pub(super) fn require_type_id(
        &self,
        type_id: TypeId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        self.require_concrete_type_id(type_id, anchor, &mut FxHashSet::default())
    }

    fn require_concrete_type_id(
        &self,
        type_id: TypeId,
        anchor: Option<HirLocation>,
        visited: &mut FxHashSet<TypeId>,
    ) -> Result<(), CompilerError> {
        if !visited.insert(type_id) {
            return Ok(());
        }

        match self.type_environment.get(type_id) {
            Some(TypeDefinition::GenericParameter(parameter)) => Err(self.error_with_hir(
                format!(
                    "Unresolved generic parameter TypeId {type_id:?} ({:?}) reached HIR validation",
                    parameter.id
                ),
                anchor,
            )),
            Some(TypeDefinition::Constructed(constructed)) => {
                for argument in &constructed.arguments {
                    self.require_concrete_type_id(*argument, anchor, visited)?;
                }

                Ok(())
            }
            Some(TypeDefinition::Function(function)) => {
                for parameter in &function.parameters {
                    self.require_concrete_type_id(parameter.type_id, anchor, visited)?;
                }

                for return_type in &function.returns {
                    self.require_concrete_type_id(*return_type, anchor, visited)?;
                }

                if let Some(error_return_type) = function.error_return {
                    self.require_concrete_type_id(error_return_type, anchor, visited)?;
                }

                Ok(())
            }
            Some(TypeDefinition::GenericInstance(instance)) => {
                for argument in &instance.arguments {
                    self.require_concrete_type_id(*argument, anchor, visited)?;
                }

                Ok(())
            }
            Some(TypeDefinition::AnonymousConstRecordMarker) => Err(self.error_with_hir(
                "compile-time-only anonymous const-record marker reached executable HIR".to_owned(),
                anchor,
            )),
            Some(_) => Ok(()),
            None => Err(self.error_with_hir(format!("Unknown HIR type id {type_id:?}"), anchor)),
        }
    }

    pub(super) fn require_same_function_cfg_owner(
        &self,
        source_block: BlockId,
        target_block: BlockId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        let Some(source_owner) = self.block_owner_by_id.get(&source_block).copied() else {
            return Err(self.error_with_hir(
                format!("Block {source_block} has no function CFG owner"),
                anchor,
            ));
        };
        let Some(target_owner) = self.block_owner_by_id.get(&target_block).copied() else {
            return Err(self.error_with_hir(
                format!("Block {target_block} has no function CFG owner"),
                anchor,
            ));
        };

        if source_owner == target_owner {
            return Ok(());
        }

        Err(self.error_with_hir(
            format!(
                "CFG edge from block {source_block} to block {target_block} crosses function boundary ({source_owner:?} -> {target_owner:?})"
            ),
            anchor,
        ))
    }

    pub(super) fn block_by_id(
        &self,
        block_id: BlockId,
    ) -> Result<&crate::compiler_frontend::hir::blocks::HirBlock, CompilerError> {
        let Some(index) = self.block_index_by_id.get(&block_id).copied() else {
            return Err(self.error_with_hir(
                format!("Unknown HIR block id {block_id:?}"),
                Some(HirLocation::Block(block_id)),
            ));
        };

        Ok(&self.module.blocks[index])
    }

    pub(super) fn require_field_owned_by(
        &self,
        field_id: FieldId,
        struct_id: StructId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        self.require_field_id(field_id, anchor)?;
        self.require_struct_id(struct_id, anchor)?;

        let Some(owner) = self.field_owner.get(&field_id).copied() else {
            return Err(self.error_with_hir(
                format!("Field {field_id:?} has no owning struct in HIR metadata"),
                anchor,
            ));
        };

        if owner == struct_id {
            return Ok(());
        }

        Err(self.error_with_hir(
            format!("Field {field_id:?} is owned by struct {owner:?}, not {struct_id:?}"),
            anchor,
        ))
    }

    // -------------------------
    //  Error Support
    // -------------------------

    // HIR validation reports compiler invariants only. Source-authored failures
    // must be rejected before this point by header, AST, or borrow validation.
    pub(super) fn error_with_text_location(
        &self,
        message: impl Into<String>,
        location: &SourceLocation,
    ) -> CompilerError {
        CompilerError::new(message, location.clone(), ErrorType::HirTransformation)
    }

    pub(super) fn error_with_hir(
        &self,
        message: impl Into<String>,
        anchor: Option<HirLocation>,
    ) -> CompilerError {
        let location = anchor
            .and_then(|hir_location| self.hir_error_location(hir_location))
            .unwrap_or_default();

        CompilerError::new(message, location, ErrorType::HirTransformation)
    }

    pub(super) fn hir_error_location(&self, location: HirLocation) -> Option<SourceLocation> {
        self.module
            .side_table
            .hir_source_location_for_hir(location)
            .or_else(|| self.module.side_table.ast_location_for_hir(location))
            .cloned()
    }
}
