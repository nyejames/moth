//! Shared lookup and error helpers for HIR validation.
//!
//! WHAT: keeps ID resolution and source-location fallback in one place for the validation family
//! modules.
//! WHY: validation should report consistent `HirTransformation` infrastructure errors anchored
//! through the HIR side table whenever possible.

use super::HirValidator;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::hir_side_table::HirLocation;
use crate::compiler_frontend::hir::ids::{
    BlockId, FieldId, FunctionId, LocalId, RegionId, StructId,
};

use rustc_hash::FxHashSet;

impl<'a> HirValidator<'a> {
    // -------------------------
    //  Resolution Helpers
    // -------------------------

    pub(super) fn require_block_id(&self, block_id: BlockId) -> Result<(), InfrastructureFailure> {
        if self.block_ids.contains(&block_id) {
            return Ok(());
        }

        Err(self.error_with_hir(format!("Unknown HIR block id {block_id:?}")))
    }

    pub(super) fn require_struct_id(
        &self,
        struct_id: StructId,
    ) -> Result<(), InfrastructureFailure> {
        if self.struct_types.contains_key(&struct_id) {
            return Ok(());
        }

        Err(self.error_with_hir(format!("Unknown HIR struct id {struct_id:?}")))
    }

    pub(super) fn require_field_id(&self, field_id: FieldId) -> Result<(), InfrastructureFailure> {
        if self.field_ids.contains(&field_id) {
            return Ok(());
        }

        Err(self.error_with_hir(format!("Unknown HIR field id {field_id:?}")))
    }

    pub(super) fn require_local_id(&self, local_id: LocalId) -> Result<(), InfrastructureFailure> {
        if self.local_types.contains_key(&local_id) {
            return Ok(());
        }

        Err(self.error_with_hir(format!("Unknown HIR local id {local_id:?}")))
    }

    pub(super) fn require_local_in_function(
        &self,
        local_id: LocalId,
        function_id: FunctionId,
    ) -> Result<(), InfrastructureFailure> {
        self.require_local_id(local_id)?;
        let Some(local_block) = self.local_block_by_id.get(&local_id).copied() else {
            return Err(self.error_with_hir(format!("Local {local_id:?} has no owning HIR block")));
        };
        let Some(local_owner) = self.block_owner_by_id.get(&local_block).copied() else {
            return Err(self.error_with_hir(format!(
                "Local {local_id:?} belongs to a block without a function owner"
            )));
        };
        if local_owner != function_id {
            return Err(self.error_with_hir(format!(
                "Local {local_id:?} belongs to function {local_owner:?}, not {function_id:?}"
            )));
        }

        Ok(())
    }

    pub(super) fn require_local_in_block(
        &self,
        local_id: LocalId,
        block_id: BlockId,
    ) -> Result<(), InfrastructureFailure> {
        self.require_local_id(local_id)?;
        if self.local_block_by_id.get(&local_id) == Some(&block_id) {
            return Ok(());
        }

        Err(self.error_with_hir(format!(
            "CFG edge destination {local_id:?} is not a local owned by target block {block_id}"
        )))
    }

    pub(super) fn require_local_not_parameter_definition(
        &self,
        local_id: LocalId,
    ) -> Result<(), InfrastructureFailure> {
        if self.parameter_local_ids.contains(&local_id) {
            return Err(self.error_with_hir(format!(
                "ABI parameter local {local_id:?} cannot be an HIR definition destination"
            )));
        }

        Ok(())
    }

    pub(super) fn require_local_for_anchor(
        &self,
        local_id: LocalId,
        anchor: Option<HirLocation>,
    ) -> Result<(), InfrastructureFailure> {
        let block_id = match anchor {
            Some(HirLocation::Block(block_id) | HirLocation::Terminator(block_id)) => {
                Some(block_id)
            }
            Some(HirLocation::Statement(statement_id)) => {
                self.statement_block_by_id.get(&statement_id).copied()
            }
            _ => None,
        };
        let function_id =
            block_id.and_then(|block_id| self.block_owner_by_id.get(&block_id).copied());
        if let Some(function_id) = function_id {
            return self.require_local_in_function(local_id, function_id);
        }

        self.require_local_id(local_id)
    }

    pub(super) fn require_region_id(
        &self,
        region_id: RegionId,
    ) -> Result<(), InfrastructureFailure> {
        if self.region_ids.contains(&region_id) {
            return Ok(());
        }

        Err(self.error_with_hir(format!("Unknown HIR region id {region_id:?}")))
    }

    pub(super) fn require_type_id(&self, type_id: TypeId) -> Result<(), InfrastructureFailure> {
        self.require_concrete_type_id(type_id, &mut FxHashSet::default())
    }

    fn require_concrete_type_id(
        &self,
        type_id: TypeId,
        visited: &mut FxHashSet<TypeId>,
    ) -> Result<(), InfrastructureFailure> {
        let definition = self.type_environment.get(type_id);

        // WHY: only composite definitions recurse, so only they need the cycle guard. Leaves
        //      return before touching `visited`, so scalar and nominal checks allocate nothing.
        //      Composites are still marked before their children are visited, so cycle
        //      detection is unchanged. Error arms need no guard: their `Err` ends the whole walk.
        if matches!(
            definition,
            Some(
                TypeDefinition::Constructed(_)
                    | TypeDefinition::Function(_)
                    | TypeDefinition::GenericInstance(_)
            )
        ) && !visited.insert(type_id)
        {
            return Ok(());
        }

        match definition {
            Some(TypeDefinition::GenericParameter(parameter)) => Err(self.error_with_hir(format!(
                "Unresolved generic parameter TypeId {type_id:?} ({:?}) reached HIR validation",
                parameter.id
            ))),
            Some(TypeDefinition::Constructed(constructed)) => {
                for argument in &constructed.arguments {
                    self.require_concrete_type_id(*argument, visited)?;
                }

                Ok(())
            }
            Some(TypeDefinition::Function(function)) => {
                for parameter in &function.parameters {
                    self.require_concrete_type_id(parameter.type_id, visited)?;
                }

                for return_type in &function.returns {
                    self.require_concrete_type_id(*return_type, visited)?;
                }

                if let Some(error_return_type) = function.error_return {
                    self.require_concrete_type_id(error_return_type, visited)?;
                }

                Ok(())
            }
            Some(TypeDefinition::GenericInstance(instance)) => {
                for argument in &instance.arguments {
                    self.require_concrete_type_id(*argument, visited)?;
                }

                Ok(())
            }
            Some(TypeDefinition::AnonymousConstRecordMarker) => Err(self.error_with_hir(
                "compile-time-only anonymous const-record marker reached executable HIR".to_owned(),
            )),
            Some(_) => Ok(()),
            None => Err(self.error_with_hir(format!("Unknown HIR type id {type_id:?}"))),
        }
    }

    pub(super) fn require_same_function_cfg_owner(
        &self,
        source_block: BlockId,
        target_block: BlockId,
    ) -> Result<(), InfrastructureFailure> {
        let Some(source_owner) = self.block_owner_by_id.get(&source_block).copied() else {
            return Err(
                self.error_with_hir(format!("Block {source_block} has no function CFG owner"))
            );
        };
        let Some(target_owner) = self.block_owner_by_id.get(&target_block).copied() else {
            return Err(
                self.error_with_hir(format!("Block {target_block} has no function CFG owner"))
            );
        };

        if source_owner == target_owner {
            return Ok(());
        }

        Err(self.error_with_hir(
            format!(
                "CFG edge from block {source_block} to block {target_block} crosses function boundary ({source_owner:?} -> {target_owner:?})"
            ),
        ))
    }

    pub(super) fn block_by_id(
        &self,
        block_id: BlockId,
    ) -> Result<&crate::compiler_frontend::hir::blocks::HirBlock, InfrastructureFailure> {
        let Some(index) = self.block_index_by_id.get(&block_id).copied() else {
            return Err(self.error_with_hir(format!("Unknown HIR block id {block_id:?}")));
        };

        Ok(&self.module.blocks[index])
    }

    pub(super) fn require_field_owned_by(
        &self,
        field_id: FieldId,
        struct_id: StructId,
    ) -> Result<(), InfrastructureFailure> {
        self.require_field_id(field_id)?;
        self.require_struct_id(struct_id)?;

        let Some(owner) = self.field_owner.get(&field_id).copied() else {
            return Err(self.error_with_hir(format!(
                "Field {field_id:?} has no owning struct in HIR metadata"
            )));
        };

        if owner == struct_id {
            return Ok(());
        }

        Err(self.error_with_hir(format!(
            "Field {field_id:?} is owned by struct {owner:?}, not {struct_id:?}"
        )))
    }

    // -------------------------
    //  Error Support
    // -------------------------

    // HIR validation reports compiler invariants only. Source-authored failures
    // must be rejected before this point by header, AST, or borrow validation.
    pub(super) fn error_with_hir(&self, message: impl Into<Box<str>>) -> InfrastructureFailure {
        InfrastructureFailure::hir_transformation(message)
    }
}
