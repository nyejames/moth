//! Builds one canonical allocation summary from a function's reachable HIR.

use super::{BorrowCheckError, BorrowChecker};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::public_call_summary::{
    CallField, CallPathType, FunctionReturnAliasSummary, PublicCallSummary,
    validate_public_call_allocation_layout,
};

#[path = "return_alias/result_paths.rs"]
mod result_paths;
pub(super) use result_paths::IndexedFunction;
pub(in crate::compiler_frontend::analysis::borrow_checker) use result_paths::ProjectedSupport;

impl BorrowChecker<'_> {
    pub(super) fn classify_function_return_alias(
        &self,
        function: &HirFunction,
        index: &IndexedFunction,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        Ok(FunctionReturnAliasSummary::Known(Box::new(
            result_paths::FunctionEvidence {
                checker: self,
                function,
                index,
            }
            .evaluate()?,
        )))
    }

    pub(super) fn validate_local_allocation_summary(
        &self,
        function: &HirFunction,
        summary: &PublicCallSummary,
    ) -> Result<(), BorrowCheckError> {
        let shape = function.return_contract.ordinary();
        let parameter_type = |index| self.local_types.get(function.params.get(index)?).copied();
        let allocation = |ty| CallPathType::hir_allocation(ty, self.type_environment);
        validate_public_call_allocation_layout(
            summary,
            shape.map(|shape| shape.success().len()),
            shape.and_then(|shape| shape.error()).is_some(),
            |path| {
                self.call_path_type(parameter_type(path.index)?, &path.path)
                    .and_then(allocation)
            },
            |error, path| {
                let shape = shape?;
                let ty = if error {
                    shape.error()?
                } else {
                    *shape.success().get(path.slot)?
                };
                self.call_path_type(ty, &path.path).and_then(allocation)
            },
            |target, relative| {
                let ty = self.call_path_type(parameter_type(target.index)?, &target.path)?;
                self.call_path_type(ty, relative).and_then(allocation)
            },
        )?;
        Ok(())
    }

    fn call_path_type(&self, mut ty: TypeId, path: &[CallField]) -> Option<TypeId> {
        for &field in path {
            ty = self.call_child_type(ty, field)?;
        }
        Some(ty)
    }

    fn call_child_type(&self, ty: TypeId, field: CallField) -> Option<TypeId> {
        field.type_child(ty, self.type_environment)
    }
}
