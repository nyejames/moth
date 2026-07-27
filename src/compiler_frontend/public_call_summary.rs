//! Shared semantic call-summary vocabulary.
//!
//! WHAT: owns the backend-neutral parameter, effect, transfer, reactive and return-alias facts
//! shared by borrow validation and the declaration-centric public-interface draft.
//! WHY: both stages consume the same semantic contract. Keeping the vocabulary at the frontend
//! boundary prevents either stage from becoming the source of a second interpretation.

use crate::compiler_frontend::compiler_errors::CompilerError;

/// The source-level access contract for one function parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicCallParameterAccess {
    Shared,
    Mutable,
    Reactive,
}

/// The mutation effect observed for one parameter's root during borrow validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicCallMutationEffect {
    NoWrite,
    Writes,
}

/// Whether final-use analysis may grant optional transfer responsibility to one parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicCallTransferEligibility {
    Ineligible,
    Eligible,
}

/// The analysis/lowering transfer category for one parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicCallTransferEffect {
    NeverConsumes,
    MayConsume,
    /// Reserved for a specialised already-proven path. Ordinary local source calls remain
    /// optional and use `MayConsume` instead.
    #[allow(dead_code)]
    AlwaysConsumes,
}

/// Reactive dependency and invalidation facts for one parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicCallReactiveEffect {
    None,
    Subscribes,
    Invalidates,
    SubscribesAndInvalidates,
}

impl PublicCallReactiveEffect {
    pub(crate) fn with_subscription(self) -> Self {
        match self {
            Self::None | Self::Subscribes => Self::Subscribes,
            Self::Invalidates => Self::SubscribesAndInvalidates,
            Self::SubscribesAndInvalidates => Self::SubscribesAndInvalidates,
        }
    }

    pub(crate) fn with_invalidation(self) -> Self {
        match self {
            Self::None | Self::Invalidates => Self::Invalidates,
            Self::Subscribes => Self::SubscribesAndInvalidates,
            Self::SubscribesAndInvalidates => Self::SubscribesAndInvalidates,
        }
    }
}

/// Owned semantic facts for one parameter, retained in source parameter order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PublicCallParameterSummary {
    pub access: PublicCallParameterAccess,
    pub mutation: PublicCallMutationEffect,
    pub transfer_eligibility: PublicCallTransferEligibility,
    pub transfer_effect: PublicCallTransferEffect,
    pub reactive_effect: PublicCallReactiveEffect,
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
}

/// Validates one concrete call summary against its declaration-owned parameter access contract.
///
/// WHAT: checks the AST signature and borrow-summary join plus the canonical shape of mutation,
/// transfer and return-alias facts before the summary crosses the public-interface boundary.
/// WHY: declared access remains stable for generic and concrete callables, while borrow validation
/// owns executable effects, including reactive propagation. This boundary rejects impossible
/// access/transfer combinations without making either producer inspect the other's source
/// representation.
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
                && parameter.transfer_eligibility == PublicCallTransferEligibility::Eligible
                && parameter.transfer_effect == PublicCallTransferEffect::MayConsume
        }
        PublicCallParameterAccess::Mutable => {
            parameter.transfer_eligibility == PublicCallTransferEligibility::Eligible
                && parameter.transfer_effect == PublicCallTransferEffect::MayConsume
        }
        PublicCallParameterAccess::Reactive => {
            parameter.mutation == PublicCallMutationEffect::NoWrite
                && parameter.transfer_eligibility == PublicCallTransferEligibility::Ineligible
                && parameter.transfer_effect == PublicCallTransferEffect::NeverConsumes
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
