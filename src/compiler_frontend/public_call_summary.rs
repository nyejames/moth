//! Shared semantic call-summary vocabulary.
//!
//! WHAT: owns backend-neutral parameter, mutation, transfer, return-alias and escaping
//! builtin-failure facts shared by semantic convergence and the public-interface draft.
//! WHY: both stages consume the same semantic contract. Keeping the vocabulary at the frontend
//! boundary prevents either stage from becoming the source of a second interpretation.

use crate::compiler_frontend::compiler_errors::CompilerError;

/// The source-level access contract for one function parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicCallParameterAccess {
    Shared,
    Mutable,
}

/// The mutation effect observed for one parameter's root during borrow validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicCallMutationEffect {
    NoWrite,
    Writes,
}

/// The analysis/lowering transfer category for one parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicCallTransferEffect {
    MayConsume,
    /// Reserved for a specialised already-proven path. Ordinary local source calls remain
    /// optional and use `MayConsume` instead.
    #[allow(dead_code)]
    AlwaysConsumes,
}

/// Owned semantic facts for one parameter, retained in source parameter order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PublicCallParameterSummary {
    pub access: PublicCallParameterAccess,
    pub mutation: PublicCallMutationEffect,
    pub transfer_effect: PublicCallTransferEffect,
}

/// User-function return alias metadata consumed by call transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FunctionReturnAliasSummary {
    Fresh,
    AliasParams(Vec<usize>),
    Unknown,
}

/// Complete semantic call contract for one local or generated function.
///
/// Parameter positions use vector order and the indices in [`FunctionReturnAliasSummary`]. No
/// donor-local HIR identity crosses this frontend semantic boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PublicCallSummary {
    pub parameters: Vec<PublicCallParameterSummary>,
    pub return_alias: FunctionReturnAliasSummary,
    /// Whether unhandled implicit builtin failure escapes the function boundary.
    /// Signature-only `false` is the initial convergence value, not proof of infallibility.
    pub escapes_builtin_failure: bool,
}

/// The result of validating one retained call-summary transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicCallSummaryTransition {
    Unchanged,
    Widened,
}

/// Validates one concrete call summary against its declaration-owned parameter access contract.
///
/// WHAT: checks the AST signature and borrow-summary join plus the canonical shape of mutation,
/// transfer and return-alias facts before the summary crosses the public-interface boundary.
/// WHY: declared access remains stable for generic and concrete callables, while borrow validation
/// owns executable effects. This boundary rejects impossible access/transfer combinations
/// without making either producer inspect the other's source representation.
pub(crate) fn validate_public_call_summary(
    declared_parameter_access: &[PublicCallParameterAccess],
    summary: &PublicCallSummary,
) -> Result<(), CompilerError> {
    if summary.parameters.len() != declared_parameter_access.len() {
        return Err(CompilerError::compiler_error(format!(
            "public call summary has {} parameter(s), but its declaration has {} parameter(s)",
            summary.parameters.len(),
            declared_parameter_access.len()
        )));
    }

    for (parameter_index, (declared_access, parameter)) in declared_parameter_access
        .iter()
        .zip(&summary.parameters)
        .enumerate()
    {
        validate_parameter_summary(parameter_index, *declared_access, parameter)?;
    }

    validate_return_alias_summary(declared_parameter_access.len(), &summary.return_alias)
}

/// Validate that a newly computed call summary preserves the finite widening order.
///
/// WHAT: checks invariant fields and the mutation, return-alias and escaping
///       builtin-failure partial orders before a summary replaces an already retained summary.
/// WHY: convergence must make progress through one explicit finite order. Silently accepting a
///      narrowing or incomparable transition would make scheduling order observable and hide an
///      error in the summary producer.
pub(in crate::compiler_frontend) fn validate_public_call_summary_transition(
    previous: &PublicCallSummary,
    next: &PublicCallSummary,
) -> Result<PublicCallSummaryTransition, CompilerError> {
    let declared_parameter_access = previous
        .parameters
        .iter()
        .map(|parameter| parameter.access)
        .collect::<Vec<_>>();
    validate_public_call_summary(&declared_parameter_access, previous)?;
    validate_public_call_summary(&declared_parameter_access, next)?;

    if previous.parameters.len() != next.parameters.len() {
        return Err(CompilerError::compiler_error(
            "public call summary transition changed parameter count",
        ));
    }

    let mut widened = false;
    if previous.escapes_builtin_failure && !next.escapes_builtin_failure {
        return Err(CompilerError::compiler_error(
            "public call summary transition narrowed escaping builtin failure",
        ));
    }
    widened |= previous.escapes_builtin_failure != next.escapes_builtin_failure;
    for (parameter_index, (previous, next)) in
        previous.parameters.iter().zip(&next.parameters).enumerate()
    {
        if previous.access != next.access {
            return Err(CompilerError::compiler_error(format!(
                "public call summary transition changed parameter {parameter_index} access"
            )));
        }
        if previous.transfer_effect != next.transfer_effect {
            return Err(CompilerError::compiler_error(format!(
                "public call summary transition changed parameter {parameter_index} transfer effect"
            )));
        }

        match (previous.mutation, next.mutation) {
            (PublicCallMutationEffect::NoWrite, PublicCallMutationEffect::NoWrite)
            | (PublicCallMutationEffect::Writes, PublicCallMutationEffect::Writes) => {}
            (PublicCallMutationEffect::NoWrite, PublicCallMutationEffect::Writes) => {
                widened = true;
            }
            (PublicCallMutationEffect::Writes, PublicCallMutationEffect::NoWrite) => {
                return Err(CompilerError::compiler_error(format!(
                    "public call summary transition narrowed parameter {parameter_index} mutation"
                )));
            }
        }
    }

    if !return_alias_is_widening(&previous.return_alias, &next.return_alias) {
        return Err(CompilerError::compiler_error(
            "public call summary transition narrowed or changed return alias incompatibly",
        ));
    }
    if previous.return_alias != next.return_alias {
        widened = true;
    }

    Ok(if widened {
        PublicCallSummaryTransition::Widened
    } else {
        PublicCallSummaryTransition::Unchanged
    })
}

fn return_alias_is_widening(
    previous: &FunctionReturnAliasSummary,
    next: &FunctionReturnAliasSummary,
) -> bool {
    match (previous, next) {
        (FunctionReturnAliasSummary::Fresh, FunctionReturnAliasSummary::Fresh)
        | (FunctionReturnAliasSummary::Unknown, FunctionReturnAliasSummary::Unknown) => true,
        (FunctionReturnAliasSummary::Fresh, FunctionReturnAliasSummary::AliasParams(_))
        | (FunctionReturnAliasSummary::Fresh, FunctionReturnAliasSummary::Unknown)
        | (FunctionReturnAliasSummary::AliasParams(_), FunctionReturnAliasSummary::Unknown) => true,
        (
            FunctionReturnAliasSummary::AliasParams(previous),
            FunctionReturnAliasSummary::AliasParams(next),
        ) => previous
            .iter()
            .all(|index| next.binary_search(index).is_ok()),
        (FunctionReturnAliasSummary::AliasParams(_), FunctionReturnAliasSummary::Fresh)
        | (FunctionReturnAliasSummary::Unknown, FunctionReturnAliasSummary::Fresh)
        | (FunctionReturnAliasSummary::Unknown, FunctionReturnAliasSummary::AliasParams(_)) => {
            false
        }
    }
}

fn validate_parameter_summary(
    parameter_index: usize,
    declared_access: PublicCallParameterAccess,
    parameter: &PublicCallParameterSummary,
) -> Result<(), CompilerError> {
    if parameter.access != declared_access {
        return Err(CompilerError::compiler_error(format!(
            "public call summary parameter {parameter_index} has {:?} access, but its declaration has {:?} access",
            parameter.access, declared_access
        )));
    }

    let valid = match declared_access {
        PublicCallParameterAccess::Shared => {
            parameter.mutation == PublicCallMutationEffect::NoWrite
                && parameter.transfer_effect == PublicCallTransferEffect::MayConsume
        }
        PublicCallParameterAccess::Mutable => {
            parameter.transfer_effect == PublicCallTransferEffect::MayConsume
        }
    };

    if !valid {
        return Err(CompilerError::compiler_error(format!(
            "public call summary parameter {parameter_index} has an invalid effect combination for {:?} access: {parameter:?}",
            declared_access,
        )));
    }

    Ok(())
}

fn validate_return_alias_summary(
    parameter_count: usize,
    return_alias: &FunctionReturnAliasSummary,
) -> Result<(), CompilerError> {
    let FunctionReturnAliasSummary::AliasParams(parameter_indices) = return_alias else {
        return Ok(());
    };

    if parameter_indices.is_empty() {
        return Err(CompilerError::compiler_error(
            "public call summary uses an empty AliasParams return; use Fresh instead",
        ));
    }

    let mut previous_index = None;
    for parameter_index in parameter_indices {
        if *parameter_index >= parameter_count {
            return Err(CompilerError::compiler_error(format!(
                "public call summary return alias references parameter index {parameter_index}, but the declaration has {parameter_count} parameter(s)"
            )));
        }
        if previous_index.is_some_and(|previous| previous >= *parameter_index) {
            return Err(CompilerError::compiler_error(format!(
                "public call summary return alias parameter indices must be strictly increasing; found {parameter_indices:?}"
            )));
        }
        previous_index = Some(*parameter_index);
    }

    Ok(())
}
