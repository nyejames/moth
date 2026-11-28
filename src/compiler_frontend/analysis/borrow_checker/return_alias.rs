//! Builds one canonical allocation summary from a function's reachable HIR.

use super::{BorrowCheckError, BorrowChecker};
use crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::public_call_summary::typed::Layout;
use crate::compiler_frontend::public_call_summary::{
    CallEndpoint, CallField, FunctionReturnAliasSummary, PublicCallSummary,
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::OnceCell;

#[path = "return_alias/result_paths.rs"]
mod result_paths;
pub(super) use result_paths::SparseFunction;

pub(super) fn build_sparse(
    checker: &BorrowChecker<'_>,
) -> Result<
    rustc_hash::FxHashMap<crate::compiler_frontend::hir::ids::FunctionId, SparseFunction>,
    BorrowCheckError,
> {
    SparseFunction::build_sparse(checker)
}
pub(in crate::compiler_frontend::analysis::borrow_checker) use result_paths::ProjectedSupport;

struct LocalCallLayout<'a, 'hir> {
    checker: &'a BorrowChecker<'hir>,
    function: &'a HirFunction,
    signature_types: OnceCell<FxHashMap<CanonicalTypeIdentity, TypeId>>,
}

impl Layout for LocalCallLayout<'_, '_> {
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
        let types = self.checker.type_environment;
        let id = types.type_id_for_canonical_identity(ty).or_else(|| {
            self.signature_types
                .get_or_init(|| {
                    let mut pending: Vec<_> = self
                        .function
                        .params
                        .iter()
                        .map(|local| self.checker.local_types[local])
                        .collect();
                    if let Some(returns) = self.function.return_contract.ordinary() {
                        pending.extend_from_slice(returns.success());
                        pending.extend(returns.error());
                    }
                    let mut seen = FxHashSet::default();
                    let mut identities = FxHashMap::default();
                    while let Some(id) = pending.pop() {
                        if seen.insert(id) {
                            if let Some(identity) = self.checker.project_type(id) {
                                identities.insert(identity, id);
                            }
                            self.children(&id, &mut |child| pending.push(child.ty));
                        }
                    }
                    identities
                })
                .get(ty)
                .copied()
        })?;
        CallEndpoint::hir(id, types)
    }

    fn matches_identity(&self, endpoint: &TypeId, identity: &CanonicalTypeIdentity) -> bool {
        self.checker.project_type(*endpoint).as_ref() == Some(identity)
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
        index: &SparseFunction,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        Ok(FunctionReturnAliasSummary::Known(Box::new(
            index.evaluate_canonical(self, function)?,
        )))
    }

    pub(super) fn validate_local_allocation_summary(
        &self,
        function: &HirFunction,
        summary: &PublicCallSummary,
    ) -> Result<(), BorrowCheckError> {
        let Some(facts) = summary.return_alias.facts() else {
            return Err(self
                .diagnostics
                .internal_error("completed call contract is still pending"));
        };
        let parameters: Vec<_> = summary.parameters.iter().map(|arg| arg.access).collect();
        let returns = function.return_contract.ordinary();
        let shape = facts.shape(
            &parameters,
            returns.map(|returns| returns.success().len()),
            returns.and_then(|returns| returns.error()).is_some(),
        );
        facts.validate_layout(
            &shape,
            &LocalCallLayout {
                checker: self,
                function,
                signature_types: OnceCell::new(),
            },
        )?;
        Ok(())
    }

    fn call_child_type(&self, ty: TypeId, field: CallField) -> Option<TypeId> {
        field.type_child(ty, self.type_environment)
    }
}
