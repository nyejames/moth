//! Function-level HIR validation.
//!
//! WHAT: checks function entries, return types and parameters.
//! WHY: borrow summaries depend on valid function metadata matching the canonical return shape.

use super::HirValidator;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::hir_side_table::HirLocation;
use rustc_hash::FxHashSet;

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

            let anchor = Some(HirLocation::Function(function.id));
            let mut parameter_ids = FxHashSet::default();
            for local in &function.params {
                if !parameter_ids.insert(*local) {
                    return Err(self.error_with_hir(
                        format!(
                            "Function {:?} lists parameter local {local:?} more than once",
                            function.id
                        ),
                        anchor,
                    ));
                }
                self.require_local_in_function(*local, function.id, anchor)?;
                if self.local_block_by_id.get(local) != Some(&function.entry) {
                    return Err(self.error_with_hir(
                        format!(
                            "Function {:?} parameter local {local:?} is not defined in its entry block",
                            function.id
                        ),
                        anchor,
                    ));
                }
            }
        }

        Ok(())
    }
}
