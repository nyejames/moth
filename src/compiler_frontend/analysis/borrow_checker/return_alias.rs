//! Builds one canonical allocation summary from a function's reachable HIR.

use super::{BorrowCheckError, BorrowChecker};
use crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::public_call_summary::typed::{
    Layout, Shape, TypedCallAllocationFacts,
};
use crate::compiler_frontend::public_call_summary::{
    CallEndpoint, CallField, CallPathType, FunctionReturnAliasSummary, PublicCallSummary,
    validate_public_call_allocation_layout,
};

#[path = "return_alias/result_paths.rs"]
mod result_paths;
pub(super) use result_paths::IndexedFunction;
pub(in crate::compiler_frontend::analysis::borrow_checker) use result_paths::ProjectedSupport;

#[allow(dead_code)]
struct LocalCallLayout<'a, 'hir, F> {
    checker: &'a BorrowChecker<'hir>,
    function: &'a HirFunction,
    resolve_stable: F,
}

impl<F: Fn(&CanonicalTypeIdentity) -> Option<TypeId>> Layout for LocalCallLayout<'_, '_, F> {
    type Type = TypeId;

    fn parameter(&self, index: usize) -> Option<CallEndpoint<TypeId>> {
        let ty = *self
            .checker
            .local_types
            .get(self.function.params.get(index)?)?;
        CallEndpoint::hir(ty, self.checker.type_environment)
    }

    fn result(&self, error: bool, slot: usize) -> Option<CallEndpoint<TypeId>> {
        let shape = self.function.return_contract.ordinary()?;
        let ty = if error {
            (slot == 0).then_some(())?;
            shape.error()?
        } else {
            *shape.success().get(slot)?
        };
        CallEndpoint::hir(ty, self.checker.type_environment)
    }

    fn child(&self, ty: &TypeId, selector: CallField) -> Option<CallEndpoint<TypeId>> {
        CallEndpoint::hir(
            self.checker.call_child_type(*ty, selector)?,
            self.checker.type_environment,
        )
    }

    fn stable(&self, ty: &CanonicalTypeIdentity) -> Option<CallEndpoint<TypeId>> {
        CallEndpoint::hir((self.resolve_stable)(ty)?, self.checker.type_environment)
    }

    fn children(&self, ty: &TypeId, visit: &mut dyn FnMut(CallEndpoint<TypeId>)) {
        let types = self.checker.type_environment;
        let _: Result<(), std::convert::Infallible> =
            CallField::for_each_child(*ty, types, |_, ty| {
                if let Some(endpoint) = CallEndpoint::hir(ty, types) {
                    visit(endpoint);
                }
                Ok(())
            });
    }
}

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

    #[allow(dead_code)]
    pub(super) fn validate_local_typed_allocation_summary(
        &self,
        function: &HirFunction,
        facts: &TypedCallAllocationFacts,
        shape: &Shape<'_>,
        resolve_stable: impl Fn(&CanonicalTypeIdentity) -> Option<TypeId>,
    ) -> Result<(), BorrowCheckError> {
        let returns = function.return_contract.ordinary();
        if function.params.len() != shape.parameters.len()
            || returns.map(|returns| returns.success().len()) != shape.success_slots
            || returns.and_then(|returns| returns.error()).is_some() != shape.error_slot
        {
            return Err(self
                .diagnostics
                .internal_error("typed call shape disagrees with its HIR signature"));
        }
        facts.validate_layout(
            shape,
            &LocalCallLayout {
                checker: self,
                function,
                resolve_stable,
            },
        )?;
        Ok(())
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
