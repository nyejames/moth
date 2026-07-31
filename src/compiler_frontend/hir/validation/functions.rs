//! Function-level HIR validation.
//!
//! WHAT: checks function entries, return types and parameters.
//! WHY: borrow summaries depend on valid function metadata matching the canonical return shape.

use super::HirValidator;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::hir_side_table::HirLocation;

impl<'a> HirValidator<'a> {
    // -------------------------
    //  Function Validation
    // -------------------------

    pub(super) fn validate_functions(&self) -> Result<(), CompilerError> {
        for function in &self.module.functions {
            self.require_block_id(function.entry, Some(HirLocation::Function(function.id)))?;
            self.require_type_id(
                function.return_type,
                Some(HirLocation::Function(function.id)),
            )?;

            for local in &function.params {
                self.require_local_id(*local, Some(HirLocation::Function(function.id)))?;
            }
        }

        Ok(())
    }
}
