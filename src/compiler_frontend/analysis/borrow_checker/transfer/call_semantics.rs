//! Call-target semantic resolution for borrow transfer.
//!
//! WHAT: maps HIR call targets to per-argument effects and result alias behavior.
//! WHY: external package access policy and user-function signature summaries are normalized once
//! here so statement transfer can stay focused on state transitions.

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalAccessKind, ExternalFunctionDef, ExternalFunctionId, ExternalReturnAlias,
};
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallParameterAccess, PublicCallParameterSummary,
    PublicCallTransferEffect, PublicCallTransferEligibility,
};

use super::BorrowTransferContext;

#[derive(Debug, Clone)]
pub(super) struct CallSemantics {
    pub(super) arg_effects: Vec<ArgEffect>,
    pub(super) return_alias: FunctionReturnAliasSummary,
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

pub(super) fn resolve_call_semantics(
    context: &BorrowTransferContext<'_>,
    target: &CallTarget,
    arg_len: usize,
    location: SourceLocation,
) -> Result<CallSemantics, BorrowCheckError> {
    match target {
        CallTarget::Local(function_id) => {
            let function_id = *function_id;
            let Some(summary) = context.public_call_summaries.get(&function_id) else {
                return Err(context.diagnostics.internal_error(
                    format!(
                        "Borrow checker is missing the public call summary for function '{}'",
                        context.diagnostics.function_name(function_id)
                    ),
                    context.diagnostics.function_error_location(function_id),
                ));
            };

            if summary.parameters.len() != arg_len {
                return Err(context.diagnostics.internal_error(
                    format!(
                        "Borrow checker found argument count mismatch for function '{}': expected {}, got {}",
                        context.diagnostics.function_name(function_id),
                        summary.parameters.len(),
                        arg_len
                    ),
                    location,
                ));
            }

            validate_return_alias_summary(
                context,
                &summary.return_alias,
                arg_len,
                location.clone(),
                &format!(
                    "user function '{}'",
                    context.diagnostics.function_name(function_id)
                ),
            )?;

            Ok(CallSemantics {
                // Ordinary immutable and mutable parameters can both receive optional transfer
                // responsibility at a proven final use. Reactive handles remain shared reads.
                arg_effects: summary
                    .parameters
                    .iter()
                    .map(parameter_arg_effect)
                    .collect(),
                return_alias: summary.return_alias.clone(),
            })
        }

        CallTarget::CrossModule(origin) => {
            let Some(summary) = context.imported_call_summaries.get(origin) else {
                return Err(context.diagnostics.internal_error(
                    format!(
                        "Borrow checker is missing the provider call summary for imported function {origin:?}"
                    ),
                    location,
                ));
            };

            if summary.parameters.len() != arg_len {
                return Err(context.diagnostics.internal_error(
                    format!(
                        "Borrow checker found argument count mismatch for imported function {origin:?}: expected {}, got {}",
                        summary.parameters.len(),
                        arg_len
                    ),
                    location,
                ));
            }

            validate_return_alias_summary(
                context,
                &summary.return_alias,
                arg_len,
                location,
                &format!("imported function {origin:?}"),
            )?;

            Ok(CallSemantics {
                arg_effects: summary
                    .parameters
                    .iter()
                    .map(parameter_arg_effect)
                    .collect(),
                return_alias: summary.return_alias.clone(),
            })
        }

        CallTarget::ModulePrivate(identity) => {
            let Some(summary) = context.module_private_call_summaries.get(identity) else {
                return Err(context.diagnostics.internal_error(
                    format!(
                        "Borrow checker is missing the call summary for module-private function {identity:?}"
                    ),
                    location,
                ));
            };

            if summary.parameters.len() != arg_len {
                return Err(context.diagnostics.internal_error(
                    format!(
                        "Borrow checker found argument count mismatch for module-private function {identity:?}: expected {}, got {}",
                        summary.parameters.len(),
                        arg_len
                    ),
                    location,
                ));
            }

            validate_return_alias_summary(
                context,
                &summary.return_alias,
                arg_len,
                location,
                &format!("module-private function {identity:?}"),
            )?;

            Ok(CallSemantics {
                arg_effects: summary
                    .parameters
                    .iter()
                    .map(parameter_arg_effect)
                    .collect(),
                return_alias: summary.return_alias.clone(),
            })
        }

        CallTarget::Generated(identity) => {
            let Some(summary) = context.generated_call_summaries.get(identity) else {
                return Err(context.diagnostics.internal_error(
                    format!(
                        "Borrow checker is missing the call summary for generated function {identity:?}"
                    ),
                    location,
                ));
            };

            if summary.parameters.len() != arg_len {
                return Err(context.diagnostics.internal_error(
                    format!(
                        "Borrow checker found argument count mismatch for generated function {identity:?}: expected {}, got {}",
                        summary.parameters.len(),
                        arg_len
                    ),
                    location,
                ));
            }

            validate_return_alias_summary(
                context,
                &summary.return_alias,
                arg_len,
                location,
                &format!("generated function {identity:?}"),
            )?;

            Ok(CallSemantics {
                arg_effects: summary
                    .parameters
                    .iter()
                    .map(parameter_arg_effect)
                    .collect(),
                return_alias: summary.return_alias.clone(),
            })
        }

        CallTarget::External(id) => {
            let host_def = resolve_host_definition(context, *id, location.clone())?;
            if host_def.parameters.len() != arg_len {
                return Err(context.diagnostics.internal_error(
                    format!(
                        "Borrow checker found argument count mismatch for host function '{}': expected {}, got {}",
                        host_def.name,
                        host_def.parameters.len(),
                        arg_len
                    ),
                    location,
                ));
            }

            let arg_effects = host_def
                .parameters
                .iter()
                .map(|param| match param.access_kind {
                    ExternalAccessKind::Shared => ArgEffect::SharedBorrow,
                    ExternalAccessKind::Mutable => ArgEffect::MutableBorrow,
                })
                .collect::<Vec<_>>();

            let return_alias = match host_def.hir_return_alias() {
                ExternalReturnAlias::Fresh => FunctionReturnAliasSummary::Fresh,
                ExternalReturnAlias::AliasArgs(indices) => {
                    validate_alias_indices(
                        context,
                        &indices,
                        arg_len,
                        location.clone(),
                        &format!("host function '{}'", host_def.name),
                    )?;
                    FunctionReturnAliasSummary::AliasParams(indices)
                }
            };

            Ok(CallSemantics {
                arg_effects,
                return_alias,
            })
        }
    }
}

fn resolve_host_definition<'a>(
    context: &'a BorrowTransferContext<'_>,
    id: ExternalFunctionId,
    location: SourceLocation,
) -> Result<&'a ExternalFunctionDef, BorrowCheckError> {
    // Host metadata is keyed by the stable ExternalFunctionId emitted into HIR.
    // Borrow checking should not silently reinterpret a missing symbol through a
    // second fallback path because that can hide registry drift.
    if let Some(definition) = context.external_package_registry.get_function_by_id(id) {
        return Ok(definition);
    }

    Err(context.diagnostics.internal_error(
        format!(
            "Borrow checker could not resolve host call target '{}'",
            id.name()
        ),
        location,
    ))
}

fn validate_alias_indices(
    context: &BorrowTransferContext<'_>,
    indices: &[usize],
    arg_len: usize,
    location: SourceLocation,
    callee_name: &str,
) -> Result<(), BorrowCheckError> {
    for index in indices {
        if *index < arg_len {
            continue;
        }

        return Err(context.diagnostics.internal_error(
            format!(
                "Borrow checker found out-of-range return-alias index {} for {} with {} argument(s)",
                index, callee_name, arg_len
            ),
            location,
        ));
    }

    Ok(())
}

fn parameter_arg_effect(parameter: &PublicCallParameterSummary) -> ArgEffect {
    match parameter.access {
        PublicCallParameterAccess::Reactive => ArgEffect::SharedBorrow,
        PublicCallParameterAccess::Shared | PublicCallParameterAccess::Mutable => {
            match (parameter.transfer_eligibility, parameter.transfer_effect) {
                (
                    PublicCallTransferEligibility::Eligible,
                    PublicCallTransferEffect::MayConsume | PublicCallTransferEffect::AlwaysConsumes,
                ) if parameter.access == PublicCallParameterAccess::Shared => {
                    ArgEffect::MayConsumeShared
                }
                (
                    PublicCallTransferEligibility::Eligible,
                    PublicCallTransferEffect::MayConsume | PublicCallTransferEffect::AlwaysConsumes,
                ) => ArgEffect::MayConsumeMutable,
                (_, _) if parameter.access == PublicCallParameterAccess::Mutable => {
                    ArgEffect::MutableBorrow
                }
                _ => ArgEffect::SharedBorrow,
            }
        }
    }
}

fn validate_return_alias_summary(
    context: &BorrowTransferContext<'_>,
    return_alias: &FunctionReturnAliasSummary,
    arg_len: usize,
    location: SourceLocation,
    callee_name: &str,
) -> Result<(), BorrowCheckError> {
    if let FunctionReturnAliasSummary::AliasParams(indices) = return_alias {
        validate_alias_indices(context, indices, arg_len, location, callee_name)?;
    }
    Ok(())
}
