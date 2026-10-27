//! Checked backend symbol and block lookup for the JS backend.
//!
//! WHAT: resolves HIR function, local, and field identifiers to their generated
//! JS names, and retrieves HIR blocks by ID.
//! WHY: these lookups are used throughout expression, statement, and function
//! lowering, so they are kept together to ensure consistent error handling.
//!
//! This module must not own source text emission, identifier generation, or
//! reachability logic. Those responsibilities belong to their focused owners.

use crate::backends::js::JsEmitter;
use crate::compiler_frontend::compiler_messages::{
    CompilerStage, compiler_errors::InfrastructureFailure,
};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::ids::{BlockId, FieldId, FunctionId, LocalId};

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn function_name(
        &self,
        function_id: FunctionId,
    ) -> Result<&str, InfrastructureFailure> {
        self.function_name_by_id
            .get(&function_id)
            .map(String::as_str)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::BackendLowering,
                    format!("JavaScript backend: missing function symbol for {function_id:?}",),
                )
            })
    }

    pub(crate) fn local_name(&self, local_id: LocalId) -> Result<&str, InfrastructureFailure> {
        self.local_name_by_id
            .get(&local_id)
            .map(String::as_str)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::BackendLowering,
                    format!("JavaScript backend: missing local symbol for {local_id:?}"),
                )
            })
    }

    pub(crate) fn field_name(&self, field_id: FieldId) -> Result<&str, InfrastructureFailure> {
        self.field_name_by_id
            .get(&field_id)
            .map(String::as_str)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::BackendLowering,
                    format!("JavaScript backend: missing field symbol for {field_id:?}"),
                )
            })
    }

    pub(crate) fn block_by_id(
        &self,
        block_id: BlockId,
    ) -> Result<&'hir HirBlock, InfrastructureFailure> {
        self.blocks_by_id.get(&block_id).copied().ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::BackendLowering,
                format!("JavaScript backend: block {block_id:?} not found in HIR module"),
            )
        })
    }
}
