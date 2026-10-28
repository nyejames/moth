//! Function-level HIR validation.
//!
//! WHAT: checks function entries, return types and parameters.
//! WHY: borrow summaries depend on valid function metadata matching the canonical return shape.

use super::HirValidator;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use rustc_hash::FxHashSet;

impl<'a> HirValidator<'a> {
    // -------------------------
    //  Function Validation
    // -------------------------

    pub(super) fn validate_functions(&self) -> Result<(), InfrastructureFailure> {
        for (target, contract) in &self.module.call_result_contracts {
            let has_signature_owner = match target {
                crate::compiler_frontend::external_packages::CallTarget::Local(_)
                | crate::compiler_frontend::external_packages::CallTarget::External(_) => true,
                crate::compiler_frontend::external_packages::CallTarget::Generated(identity) => {
                    self.module
                        .function_ids_by_generated
                        .contains_key(identity.as_ref())
                }
                crate::compiler_frontend::external_packages::CallTarget::ModulePrivate(
                    identity,
                ) => self
                    .module
                    .function_ids_by_private_origin
                    .contains_key(identity),
                crate::compiler_frontend::external_packages::CallTarget::CrossModule(identity) => {
                    self.module.function_ids_by_origin.contains_key(identity)
                }
            };
            if has_signature_owner {
                return Err(
                    self.error_with_hir("Call result contract mirrors a reachable signature owner")
                );
            }
            for return_type in contract.returns.iter() {
                self.require_type_id(*return_type)?;
            }
            if let Some(error_type) = contract.error_return {
                self.require_type_id(error_type)?;
            }
        }
        for function in &self.module.functions {
            self.require_block_id(function.entry)?;
            for return_type in function.returns.iter() {
                self.require_type_id(*return_type)?;
            }
            if let Some(error_type) = function.error_return {
                self.require_type_id(error_type)?;
            }

            let mut parameter_ids = FxHashSet::default();
            for local in &function.params {
                if !parameter_ids.insert(*local) {
                    return Err(self.error_with_hir(format!(
                        "Function {:?} lists parameter local {local:?} more than once",
                        function.id
                    )));
                }
                self.require_local_in_function(*local, function.id)?;
                if self.local_block_by_id.get(local) != Some(&function.entry) {
                    return Err(self.error_with_hir(format!(
                        "Function {:?} parameter local {local:?} is not defined in its entry block",
                        function.id
                    )));
                }
            }
        }

        Ok(())
    }
}
