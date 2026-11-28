//! Call-target semantic resolution for borrow transfer.
//!
//! WHAT: maps HIR call targets to per-argument effects and result alias behavior.
//! WHY: external package access policy and user-function signature summaries are normalized once
//! here so statement transfer can stay focused on state transitions.

use super::{
    BorrowDiagnostics, ExternalPackageRegistry, FunctionId, FxHashMap, GeneratedFunctionIdentity,
    ModulePrivateExecutableIdentity, OriginFunctionId, PublicCallSummary,
};
use crate::compiler_frontend::analysis::borrow_checker::{BorrowCheckError, BorrowChecker};
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalAccessKind, ExternalFunctionDef, ExternalReturnAlias, ExternalReturnSlot,
};
use crate::compiler_frontend::public_call_summary::typed::{Lane, TypedCallAllocationFacts};
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallParameterAccess, PublicCallParameterSummary,
    PublicCallTransferEffect,
};

use super::BorrowTransferContext;

pub(super) struct CallSemantics<'a> {
    call: BorrowedCall<'a>,
    arg_len: usize,
}

impl<'a> CallSemantics<'a> {
    pub(super) fn arg_effects(&self) -> impl ExactSizeIterator<Item = ArgEffect> + '_ {
        let call = self.call;
        (0..self.arg_len).map(move |index| call.arg_effect(index))
    }

    pub(super) fn returns(&self) -> CallReturns<'a> {
        self.call.returns()
    }
}

#[derive(Debug, Clone, Copy)]
pub(in super::super) enum CallReturns<'a> {
    Summary(&'a FunctionReturnAliasSummary),
    External(&'a [ExternalReturnSlot]),
    Fresh,
}

/// The executable prefix is observed even when there is no continuation.
/// Pending is not a settled proof of divergence.
#[allow(dead_code)]
pub(super) enum TypedContinuation<'a> {
    Deferred,
    NoContinuation,
    Match(&'a Lane),
}

#[allow(dead_code)]
pub(super) fn classify_typed_continuation<'a>(
    facts: Option<&'a TypedCallAllocationFacts>,
    terminal: bool,
    error: bool,
    mandatory: bool,
    diagnostics: &BorrowDiagnostics<'_>,
) -> Result<TypedContinuation<'a>, BorrowCheckError> {
    let Some(facts) = facts else {
        return if mandatory {
            Err(diagnostics
                .internal_error("Unsolved typed call reached mandatory borrow validation"))
        } else {
            Ok(TypedContinuation::Deferred)
        };
    };
    let lane = facts.lane(error);
    Ok(if terminal || !lane.can_return() {
        TypedContinuation::NoContinuation
    } else {
        TypedContinuation::Match(lane)
    })
}

/// Per-argument effect contract consumed by transfer.
///
/// Why this exists:
/// `~` call parameters are not always plain mutable borrows. They can either
/// remain a borrow or receive optional transfer responsibility based on last-use facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ArgEffect {
    SharedBorrow,
    MutableBorrow,
    MayConsumeShared,
    MayConsumeMutable,
}

impl ArgEffect {
    pub(super) fn may_write(self) -> bool {
        matches!(self, Self::MutableBorrow | Self::MayConsumeMutable)
    }
}

pub(super) fn resolve_call_semantics<'a>(
    context: &BorrowTransferContext<'a>,
    target: &CallTarget,
    arg_len: usize,
) -> Result<CallSemantics<'a>, BorrowCheckError> {
    let call = CallSignatures {
        registry: context.external_package_registry,
        local: context.public_call_summaries,
        imported: context.imported_call_summaries,
        private: context.module_private_call_summaries,
        generated: context.generated_call_summaries,
    }
    .resolve(target, arg_len, &context.diagnostics)?;
    Ok(CallSemantics { call, arg_len })
}

pub(in super::super) struct CallSignatures<'a> {
    registry: &'a ExternalPackageRegistry,
    local: &'a FxHashMap<FunctionId, PublicCallSummary>,
    imported: &'a FxHashMap<OriginFunctionId, PublicCallSummary>,
    private: &'a FxHashMap<ModulePrivateExecutableIdentity, PublicCallSummary>,
    generated: &'a FxHashMap<GeneratedFunctionIdentity, PublicCallSummary>,
}

impl BorrowChecker<'_> {
    pub(in super::super) fn call_signatures(&self) -> CallSignatures<'_> {
        CallSignatures {
            registry: self.external_package_registry,
            local: &self.public_call_summaries,
            imported: &self.module.imported_call_summaries,
            private: &self.module.module_private_call_summaries,
            generated: &self.module.generated_call_summaries,
        }
    }
}

#[derive(Clone, Copy)]
pub(in super::super) enum BorrowedCall<'a> {
    Host(&'a ExternalFunctionDef),
    User(&'a PublicCallSummary),
}

impl<'a> CallSignatures<'a> {
    pub(in super::super) fn summary(&self, target: &CallTarget) -> Option<&'a PublicCallSummary> {
        match target {
            CallTarget::Local(id) => self.local.get(id),
            CallTarget::CrossModule(origin) => self.imported.get(origin),
            CallTarget::ModulePrivate(identity) => self.private.get(identity),
            CallTarget::Generated(identity) => self.generated.get(identity.as_ref()),
            CallTarget::External(_) => None,
        }
    }

    pub(in super::super) fn resolve(
        &self,
        target: &CallTarget,
        arg_len: usize,
        diagnostics: &BorrowDiagnostics<'_>,
    ) -> Result<BorrowedCall<'a>, BorrowCheckError> {
        if let CallTarget::External(id) = target {
            let host = self.registry.get_function_by_id(*id).ok_or_else(|| {
                diagnostics.internal_error(format!(
                    "Borrow checker could not resolve host call target '{}'",
                    id.name(),
                ))
            })?;
            if host.parameters.len() != arg_len {
                return Err(diagnostics.internal_error(format!(
                    "Borrow checker found argument count mismatch for host function '{}': expected {}, got {}",
                    host.name, host.parameters.len(), arg_len,
                )));
            }
            for slot in host.hir_return_sources() {
                if let ExternalReturnAlias::AliasArgs(indices) = &slot.alias
                    && let Some(index) = indices.iter().find(|&&index| index >= arg_len)
                {
                    return Err(diagnostics.internal_error(format!(
                        "Borrow checker found out-of-range return-alias index {} for {} with {} argument(s)",
                        index, host.name, arg_len,
                    )));
                }
            }
            return Ok(BorrowedCall::Host(host));
        }
        let summary = self.summary(target).ok_or_else(|| {
            diagnostics.internal_error(format!(
                "Borrow checker is missing the public call summary for {target:?}"
            ))
        })?;
        if summary.parameters.len() != arg_len {
            return Err(diagnostics.internal_error(format!(
                "Borrow checker found argument count mismatch for {target:?}: expected {}, got {}",
                summary.parameters.len(),
                arg_len,
            )));
        }
        Ok(BorrowedCall::User(summary))
    }
}

impl<'a> BorrowedCall<'a> {
    fn arg_effect(self, index: usize) -> ArgEffect {
        match self {
            Self::Host(host) => match host.parameters[index].access_kind {
                ExternalAccessKind::Shared => ArgEffect::SharedBorrow,
                ExternalAccessKind::Mutable => ArgEffect::MutableBorrow,
            },
            Self::User(summary) => parameter_arg_effect(&summary.parameters[index]),
        }
    }

    pub(in super::super) fn is_mutable(self, index: usize) -> bool {
        self.arg_effect(index).may_write()
    }

    pub(in super::super) fn returns(self) -> CallReturns<'a> {
        match self {
            Self::Host(host) => CallReturns::External(host.hir_return_sources()),
            Self::User(summary) => CallReturns::Summary(&summary.return_alias),
        }
    }
}

fn parameter_arg_effect(parameter: &PublicCallParameterSummary) -> ArgEffect {
    match (parameter.access, parameter.transfer_effect) {
        (
            PublicCallParameterAccess::Shared,
            PublicCallTransferEffect::MayConsume | PublicCallTransferEffect::AlwaysConsumes,
        ) => ArgEffect::MayConsumeShared,
        (
            PublicCallParameterAccess::Mutable,
            PublicCallTransferEffect::MayConsume | PublicCallTransferEffect::AlwaysConsumes,
        ) => ArgEffect::MayConsumeMutable,
    }
}
