//! Call-target semantic resolution for borrow transfer.
//!
//! WHAT: maps HIR call targets to per-argument effects and result alias behavior.
//! WHY: external package access policy and user-function signature summaries are normalized once
//! here so statement transfer can stay focused on state transitions.

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalAccessKind, ExternalFunctionDef, ExternalFunctionId, ExternalReturnAlias,
    ExternalReturnSlot,
};
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallParameterAccess, PublicCallParameterSummary,
    PublicCallTransferEffect,
};

use super::BorrowTransferContext;

#[derive(Debug, Clone)]
pub(super) struct CallSemantics<'a> {
    pub(super) arg_effects: Vec<ArgEffect>,
    pub(super) returns: CallReturns<'a>,
}

#[derive(Debug, Clone, Copy)]
pub(in super::super) enum CallReturns<'a> {
    Summary(&'a FunctionReturnAliasSummary),
    External(&'a [ExternalReturnSlot]),
    Fresh,
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

pub(super) fn resolve_call_semantics<'a>(
    context: &BorrowTransferContext<'a>,
    target: &CallTarget,
    arg_len: usize,
) -> Result<CallSemantics<'a>, BorrowCheckError> {
    let summary = match target {
        CallTarget::Local(id) => context.public_call_summaries.get(id),
        CallTarget::CrossModule(origin) => context.imported_call_summaries.get(origin),
        CallTarget::ModulePrivate(identity) => context.module_private_call_summaries.get(identity),
        CallTarget::Generated(identity) => context.generated_call_summaries.get(identity.as_ref()),
        CallTarget::External(id) => {
            let host = resolve_host_definition(context, *id)?;
            if host.parameters.len() != arg_len {
                return Err(context.diagnostics.internal_error(format!(
                    "Borrow checker found argument count mismatch for host function '{}': expected {}, got {}",
                    host.name, host.parameters.len(), arg_len,
                )));
            }
            let slots = host.hir_return_sources();
            for slot in slots {
                if let ExternalReturnAlias::AliasArgs(indices) = &slot.alias {
                    validate_alias_indices(context, indices, arg_len, &host.name)?;
                }
            }
            return Ok(CallSemantics {
                arg_effects: host
                    .parameters
                    .iter()
                    .map(|param| match param.access_kind {
                        ExternalAccessKind::Shared => ArgEffect::SharedBorrow,
                        ExternalAccessKind::Mutable => ArgEffect::MutableBorrow,
                    })
                    .collect(),
                returns: CallReturns::External(slots),
            });
        }
    };
    let Some(summary) = summary else {
        return Err(context.diagnostics.internal_error(format!(
            "Borrow checker is missing the public call summary for {target:?}"
        )));
    };
    if summary.parameters.len() != arg_len {
        return Err(context.diagnostics.internal_error(format!(
            "Borrow checker found argument count mismatch for {target:?}: expected {}, got {}",
            summary.parameters.len(),
            arg_len,
        )));
    }
    Ok(CallSemantics {
        arg_effects: summary
            .parameters
            .iter()
            .map(parameter_arg_effect)
            .collect(),
        returns: CallReturns::Summary(&summary.return_alias),
    })
}

fn resolve_host_definition<'a>(
    context: &BorrowTransferContext<'a>,
    id: ExternalFunctionId,
) -> Result<&'a ExternalFunctionDef, BorrowCheckError> {
    // Host metadata is keyed by the stable ExternalFunctionId emitted into HIR.
    // Borrow checking should not silently reinterpret a missing symbol through a
    // second fallback path because that can hide registry drift.
    if let Some(definition) = context.external_package_registry.get_function_by_id(id) {
        return Ok(definition);
    }

    Err(context.diagnostics.internal_error(format!(
        "Borrow checker could not resolve host call target '{}'",
        id.name()
    )))
}

fn validate_alias_indices(
    context: &BorrowTransferContext<'_>,
    indices: &[usize],
    arg_len: usize,
    callee_name: &str,
) -> Result<(), BorrowCheckError> {
    for index in indices {
        if *index < arg_len {
            continue;
        }

        return Err(context.diagnostics.internal_error(format!(
            "Borrow checker found out-of-range return-alias index {} for {} with {} argument(s)",
            index, callee_name, arg_len
        )));
    }

    Ok(())
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
