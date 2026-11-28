//! Shared semantic call-summary vocabulary.
//!
//! WHAT: owns backend-neutral parameter, mutation, transfer, return-alias and escaping
//! builtin-failure facts shared by semantic convergence and the public-interface draft.
//! WHY: both stages consume the same semantic contract. Keeping the vocabulary at the frontend
//! boundary prevents either stage from becoming the source of a second interpretation.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::datatypes::definitions::ChoiceVariantPayloadDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::expressions::OPTION_SOME_VARIANT_INDEX;
use crate::compiler_frontend::hir::hir_datatypes::classify_hir_type;
use crate::compiler_frontend::hir::module::HirChoice;
use crate::compiler_frontend::hir::structs::HirStruct;

#[allow(dead_code)] // The payload is exercised directly before it replaces the live contract.
pub(in crate::compiler_frontend) mod typed;

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

/// Allocation evidence at the call boundary. Pending is the fixed-point seed, not fresh
/// storage or an effect-free returning call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FunctionReturnAliasSummary {
    Pending,
    Known(Box<CallAllocationFacts>),
}

impl FunctionReturnAliasSummary {
    /// A pending contract and two bottom lanes expose no continuation or allocation facts
    /// to the internal fixed point. They differ at the mandatory validation boundary.
    pub(crate) fn same_allocation_transfer_as(&self, next: &Self) -> bool {
        let both_lanes_unreachable =
            |facts: &CallAllocationFacts| !facts.success.can_return() && !facts.error.can_return();
        match (self, next) {
            (Self::Pending, Self::Known(facts)) | (Self::Known(facts), Self::Pending) => {
                both_lanes_unreachable(facts)
            }
            _ => self == next,
        }
    }
}

#[allow(dead_code)]
pub(crate) fn same_typed_allocation_transfer_as(
    previous: Option<&typed::TypedCallAllocationFacts>,
    next: Option<&typed::TypedCallAllocationFacts>,
) -> bool {
    match (previous, next) {
        (None, Some(facts)) | (Some(facts), None) => facts.is_transfer_bottom(),
        _ => previous == next,
    }
}

#[allow(dead_code)]
pub(crate) fn validate_typed_allocation_transition(
    previous: Option<&typed::TypedCallAllocationFacts>,
    next: Option<&typed::TypedCallAllocationFacts>,
    shape: &typed::Shape<'_>,
) -> Result<PublicCallSummaryTransition, InfrastructureFailure> {
    match (previous, next) {
        (Some(previous), Some(next)) => previous.validate_transition(next, shape)?,
        (None, Some(next)) => next.validate(shape)?,
        (Some(_), None) => return Err(allocation_error("solved typed call became pending")),
        (None, None) => {}
    }
    Ok(if previous == next {
        PublicCallSummaryTransition::Unchanged
    } else {
        PublicCallSummaryTransition::Widened
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CallAllocationFacts {
    pub success: CallLaneSummary,
    pub error: CallLaneSummary,
}

/// A lane with allocation evidence has sorted, duplicate-free result, sharing and store atoms.
/// Structural return flow is tracked separately: a missing allocation holder is not proof that
/// the lane cannot execute.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CallLaneSummary {
    pub reachable: bool,
    /// The HIR and call contracts allow this outcome even though extraction found no complete
    /// allocation path. The precise case (`reachable`) implies the same structural fact.
    pub structural_only: bool,
    pub sources: Vec<CallPathSource>,
    pub sharing: Vec<CallSharing>,
    pub stores: Vec<CallPathSource>,
}

impl CallLaneSummary {
    pub(crate) fn can_return(&self) -> bool {
        self.reachable || self.structural_only
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CallPathSource {
    pub output: CallOutputPath,
    pub source: CallAllocationSource,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CallOutputPath {
    Result(CallResultPath),
    /// Store into the caller's mutable argument cell when `target.path` is empty. A
    /// nonempty target instead names a cell in the allocation passed at call entry.
    /// `path` selects within the newly stored value, not within the old allocation.
    Store {
        target: CallParameterPath,
        path: Vec<CallField>,
    },
}

impl CallOutputPath {
    /// An element or key selector can stand for more than one allocation.
    pub(crate) fn has_collapsed_selector(&self) -> bool {
        match self {
            Self::Result(result) => result.path.iter().any(|step| step.is_collapsed()),
            Self::Store { target, path } => target
                .path
                .iter()
                .chain(path)
                .any(|step| step.is_collapsed()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CallResultPath {
    pub slot: usize,
    pub path: Vec<CallField>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CallParameterPath {
    pub index: usize,
    pub path: Vec<CallField>,
}

/// Ordinals in the resolved declaration layout, never donor-local HIR identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum CallField {
    Field(u32),
    Payload { variant: u32, field: u32 },
    Elem,
    MapKey,
}

/// The equivalent selector in the current HIR layout.
pub(crate) enum CallFieldHir {
    Field(u32),
    /// Valid ordinal whose enclosing type was not materialised into a local HIR struct.
    UnmaterializedField(u32),
    Payload {
        variant: u32,
        field: u32,
    },
    Elem,
    MapKey,
}

impl CallField {
    /// An element or map-key selector stands for every element or key, not one child.
    pub(crate) fn is_collapsed(self) -> bool {
        matches!(self, Self::Elem | Self::MapKey)
    }

    /// Visit the selector and declared type of every immediate child of `ty`, in declaration
    /// order. This is the one enumeration of call selectors; summary extraction and live opaque
    /// transfer both use it.
    pub(crate) fn for_each_child<E>(
        ty: TypeId,
        types: &TypeEnvironment,
        mut visit: impl FnMut(Self, TypeId) -> Result<(), E>,
    ) -> Result<(), E> {
        if let Some(element) = types.collection_element_type(ty) {
            visit(Self::Elem, element)?;
        } else if let Some(map) = types.map_shape(ty) {
            visit(Self::MapKey, map.key_type)?;
            visit(Self::Elem, map.value_type)?;
        } else if let Some(inner) = types.option_inner_type(ty) {
            visit(
                Self::Payload {
                    variant: OPTION_SOME_VARIANT_INDEX as u32,
                    field: 0,
                },
                inner,
            )?;
        } else if let Some(fields) = types.fields_for(ty) {
            for (ordinal, field) in fields.iter().enumerate() {
                visit(Self::Field(ordinal as u32), field.type_id)?;
            }
        } else if let Some(variants) = types.variants_for(ty) {
            for (variant, definition) in variants.iter().enumerate() {
                if let ChoiceVariantPayloadDefinition::Record { fields } = &definition.payload {
                    for (field, definition) in fields.iter().enumerate() {
                        visit(
                            Self::Payload {
                                variant: variant as u32,
                                field: field as u32,
                            },
                            definition.type_id,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Resolve a canonical selector even when a concrete nominal has not acquired local HIR IDs.
    pub(crate) fn type_child(self, ty: TypeId, types: &TypeEnvironment) -> Option<TypeId> {
        match self {
            Self::Field(ordinal) => Some(types.fields_for(ty)?.get(ordinal as usize)?.type_id),
            Self::Payload { variant, field } => {
                if let Some(inner) = types.option_inner_type(ty) {
                    return (variant as usize == OPTION_SOME_VARIANT_INDEX && field == 0)
                        .then_some(inner);
                }
                let payload = &types.variants_for(ty)?.get(variant as usize)?.payload;
                let ChoiceVariantPayloadDefinition::Record { fields } = payload else {
                    return None;
                };
                Some(fields.get(field as usize)?.type_id)
            }
            Self::Elem => types
                .collection_element_type(ty)
                .or_else(|| types.map_shape(ty).map(|map| map.value_type)),
            Self::MapKey => Some(types.map_shape(ty)?.key_type),
        }
    }

    pub(crate) fn hir_child(
        self,
        ty: TypeId,
        types: &TypeEnvironment,
        structs: &[HirStruct],
        choices: &[HirChoice],
    ) -> Option<(CallFieldHir, TypeId)> {
        let structure = match self {
            Self::Field(_) => structs.iter().find(|layout| layout.frontend_type_id == ty),
            _ => None,
        };
        let choice = match self {
            Self::Payload { .. } => choices.iter().find(|layout| layout.frontend_type_id == ty),
            _ => None,
        };
        self.hir_child_with_layout(ty, types, structure, choice)
    }

    pub(crate) fn hir_child_with_layout(
        self,
        ty: TypeId,
        types: &TypeEnvironment,
        structure: Option<&HirStruct>,
        choice: Option<&HirChoice>,
    ) -> Option<(CallFieldHir, TypeId)> {
        match self {
            Self::Field(ordinal) => {
                if let Some(layout) = structure {
                    let field = layout.fields.get(ordinal as usize)?;
                    Some((CallFieldHir::Field(field.id.0), field.ty))
                } else {
                    Some((
                        CallFieldHir::UnmaterializedField(ordinal),
                        self.type_child(ty, types)?,
                    ))
                }
            }
            Self::Payload { variant, field } => {
                let child = if let Some(layout) = choice {
                    layout
                        .variants
                        .get(variant as usize)?
                        .fields
                        .get(field as usize)?
                        .ty
                } else {
                    self.type_child(ty, types)?
                };
                Some((CallFieldHir::Payload { variant, field }, child))
            }
            Self::Elem => Some((CallFieldHir::Elem, self.type_child(ty, types)?)),
            Self::MapKey => Some((CallFieldHir::MapKey, self.type_child(ty, types)?)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CallAllocationSource {
    Parameter(CallParameterPath),
    /// Fresh at this endpoint; only a sharing row says which other endpoint shares it.
    Fresh,
    /// Typed opaque topology of storage that cannot retain an input allocation.
    OpaqueFresh,
}

/// One same-execution sharing atom. Separate rows are a disjunction, not a transitive relation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CallSharing {
    pub left: CallOutputPath,
    pub right: CallOutputPath,
    pub condition: CallSharingCondition,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CallSharingCondition {
    Always,
    Parameters(CallParameterPath, CallParameterPath),
}

/// Complete semantic call contract for one local or generated function.
///
/// Parameter positions use vector order and the indices in [`FunctionReturnAliasSummary`]. No
/// donor-local HIR identity crosses this frontend semantic boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PublicCallSummary {
    pub parameters: Vec<PublicCallParameterSummary>,
    /// Result allocation sources, same-execution sharing and retained stores by return lane.
    pub return_alias: FunctionReturnAliasSummary,
    /// Whether unhandled implicit builtin failure escapes the function boundary.
    /// Signature-only `false` is the initial convergence value, not proof of infallibility.
    pub escapes_builtin_failure: bool,
    /// Whether any statically selected fast numeric operation or fast contract is reachable.
    ///
    /// WHAT: the `$safe_math` guarantee bit. `false` after constraint convergence is proof for a
    ///       Moth-source callee; the initial `false` is a seed, not evidence.
    /// WHY: guarantees ride the existing call summary and interface equality instead of a
    ///      separate fingerprint type. Constraint convergence widens it monotonically.
    pub uses_fast_semantics: bool,
    /// Whether a reachable Moth semantic trap can execute in the function.
    pub may_trap: bool,
    /// Whether a recoverable builtin error can escape the function through any channel.
    ///
    /// Deliberately independent of [`Self::escapes_builtin_failure`]: that field routes private
    /// failure lanes, while this field records the post-lane proof-based escape fact.
    pub may_fail: bool,
}

/// The result of validating one retained call-summary transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicCallSummaryTransition {
    Unchanged,
    Widened,
}

/// Validates one concrete call summary against its declaration-owned parameter access contract.
///
/// Checks the AST signature and borrow-summary access, mutation and transfer contract.
/// WHY: declared access remains stable for generic and concrete callables, while borrow validation
/// owns executable effects. This boundary rejects impossible access/transfer combinations
/// without making either producer inspect the other's source representation.
pub(crate) fn validate_public_call_parameter_contract(
    declared_parameter_access: impl ExactSizeIterator<Item = PublicCallParameterAccess>,
    summary: &PublicCallSummary,
) -> Result<(), InfrastructureFailure> {
    if summary.parameters.len() != declared_parameter_access.len() {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!(
                "public call summary has {} parameter(s), but its declaration has {} parameter(s)",
                summary.parameters.len(),
                declared_parameter_access.len()
            ),
        ));
    }

    for (parameter_index, (declared_access, parameter)) in declared_parameter_access
        .zip(&summary.parameters)
        .enumerate()
    {
        validate_parameter_summary(parameter_index, declared_access, parameter)?;
    }

    Ok(())
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
) -> Result<PublicCallSummaryTransition, InfrastructureFailure> {
    validate_return_alias_summary(previous)?;
    validate_return_alias_summary(next)?;

    if previous.parameters.len() != next.parameters.len() {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "public call summary transition changed parameter count",
        ));
    }

    let mut widened = false;
    if previous.escapes_builtin_failure && !next.escapes_builtin_failure {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "public call summary transition narrowed escaping builtin failure",
        ));
    }
    widened |= previous.escapes_builtin_failure != next.escapes_builtin_failure;

    // Constraint evidence is one more finite false->true order. A narrowing transition would mean
    // a later pass replaced proved-absent evidence with blank, so reject it like any other
    // narrowing instead of letting pass order change the guarantee.
    for (name, previous, next) in [
        (
            "fast semantics",
            previous.uses_fast_semantics,
            next.uses_fast_semantics,
        ),
        ("trap", previous.may_trap, next.may_trap),
        ("recoverable failure", previous.may_fail, next.may_fail),
    ] {
        if previous && !next {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!("public call summary transition narrowed {name}"),
            ));
        }
        widened |= previous != next;
    }

    for (parameter_index, (previous, next)) in
        previous.parameters.iter().zip(&next.parameters).enumerate()
    {
        validate_parameter_summary(parameter_index, previous.access, previous)?;
        validate_parameter_summary(parameter_index, previous.access, next)?;
        if previous.transfer_effect != next.transfer_effect {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "public call summary transition changed parameter {parameter_index} transfer effect"
                ),
            ));
        }

        match (previous.mutation, next.mutation) {
            (PublicCallMutationEffect::NoWrite, PublicCallMutationEffect::NoWrite)
            | (PublicCallMutationEffect::Writes, PublicCallMutationEffect::Writes) => {}
            (PublicCallMutationEffect::NoWrite, PublicCallMutationEffect::Writes) => {
                widened = true;
            }
            (PublicCallMutationEffect::Writes, PublicCallMutationEffect::NoWrite) => {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    format!(
                        "public call summary transition narrowed parameter {parameter_index} mutation"
                    ),
                ));
            }
        }
    }

    if !return_alias_is_widening(&previous.return_alias, &next.return_alias) {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
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
    use FunctionReturnAliasSummary::{Known, Pending};

    match (previous, next) {
        (Pending, _) => true,
        (Known(previous), Known(next)) => {
            lane_is_widening(&previous.success, &next.success)
                && lane_is_widening(&previous.error, &next.error)
        }
        (Known(_), Pending) => false,
    }
}

fn lane_is_widening(previous: &CallLaneSummary, next: &CallLaneSummary) -> bool {
    (!previous.can_return() || next.can_return())
        && (!previous.reachable
            || (next.reachable
                && sorted_subset(&previous.sources, &next.sources)
                && sorted_subset(&previous.sharing, &next.sharing)
                && sorted_subset(&previous.stores, &next.stores)))
}

fn sorted_subset<T: Ord>(previous: &[T], next: &[T]) -> bool {
    let mut next = next.iter();
    previous
        .iter()
        .all(|atom| next.by_ref().find(|candidate| *candidate >= atom) == Some(atom))
}

fn validate_parameter_summary(
    parameter_index: usize,
    declared_access: PublicCallParameterAccess,
    parameter: &PublicCallParameterSummary,
) -> Result<(), InfrastructureFailure> {
    if parameter.access != declared_access {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!(
                "public call summary parameter {parameter_index} has {:?} access, but its declaration has {:?} access",
                parameter.access, declared_access
            ),
        ));
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
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!(
                "public call summary parameter {parameter_index} has an invalid effect combination for {:?} access: {parameter:?}",
                declared_access,
            ),
        ));
    }

    Ok(())
}

fn validate_return_alias_summary(summary: &PublicCallSummary) -> Result<(), InfrastructureFailure> {
    match &summary.return_alias {
        FunctionReturnAliasSummary::Known(facts) => {
            validate_lane_atoms(&facts.success, &summary.parameters)?;
            validate_lane_atoms(&facts.error, &summary.parameters)?;
        }
        FunctionReturnAliasSummary::Pending => {}
    }
    Ok(())
}

fn validate_lane_atoms(
    lane: &CallLaneSummary,
    parameters: &[PublicCallParameterSummary],
) -> Result<(), InfrastructureFailure> {
    if !lane.reachable
        && !(lane.sources.is_empty() && lane.stores.is_empty() && lane.sharing.is_empty())
    {
        return Err(allocation_error("unreachable lane has allocation facts"));
    }
    if !strictly_sorted(&lane.sources)
        || !strictly_sorted(&lane.stores)
        || !strictly_sorted(&lane.sharing)
    {
        return Err(allocation_error(
            "lane source, sharing and store rows must each be strictly increasing",
        ));
    }

    for row in &lane.sources {
        if !matches!(row.output, CallOutputPath::Result(_)) {
            return Err(allocation_error("result source row uses a store endpoint"));
        }
        validate_parent_output(&lane.sources, &row.output)?;
    }
    for row in &lane.stores {
        let CallOutputPath::Store { target, .. } = &row.output else {
            return Err(allocation_error("store row uses a result endpoint"));
        };
        let receiver = validate_parameter_index(target.index, parameters)?;
        if receiver.access != PublicCallParameterAccess::Mutable {
            return Err(allocation_error(format!(
                "store target {target:?} must name a mutable parameter or its projection"
            )));
        }
        validate_parent_output(&lane.stores, &row.output)?;
    }
    for row in lane.sources.iter().chain(&lane.stores) {
        if let CallAllocationSource::Parameter(parameter) = &row.source {
            validate_parameter_index(parameter.index, parameters)?;
        }
    }
    for sharing in &lane.sharing {
        if sharing.left >= sharing.right {
            return Err(allocation_error(format!(
                "sharing endpoints are not in strict canonical order: {sharing:?}"
            )));
        }
        let contains_output = |output: &CallOutputPath| {
            let rows = match output {
                CallOutputPath::Result(_) => &lane.sources,
                CallOutputPath::Store { .. } => &lane.stores,
            };
            rows.binary_search_by(|row| row.output.cmp(output)).is_ok()
        };
        if !contains_output(&sharing.left) || !contains_output(&sharing.right) {
            return Err(allocation_error(format!(
                "sharing endpoint has no source in its lane: {sharing:?}"
            )));
        }
        if let CallSharingCondition::Parameters(left, right) = &sharing.condition {
            if left > right {
                return Err(allocation_error(format!(
                    "sharing parameter condition is not in canonical order: {sharing:?}"
                )));
            }
            validate_parameter_index(left.index, parameters)?;
            validate_parameter_index(right.index, parameters)?;
        }
    }
    Ok(())
}

fn validate_parent_output(
    rows: &[CallPathSource],
    output: &CallOutputPath,
) -> Result<(), InfrastructureFailure> {
    let path = match output {
        CallOutputPath::Result(result) => &result.path,
        CallOutputPath::Store { path, .. } => path,
    };
    let Some((_, parent)) = path.split_last() else {
        return Ok(());
    };
    let parent_exists = rows
        .binary_search_by(|row| match (&row.output, output) {
            (CallOutputPath::Result(candidate), CallOutputPath::Result(result)) => candidate
                .slot
                .cmp(&result.slot)
                .then_with(|| candidate.path.as_slice().cmp(parent)),
            (
                CallOutputPath::Store {
                    target: candidate,
                    path,
                },
                CallOutputPath::Store { target, .. },
            ) => candidate
                .cmp(target)
                .then_with(|| path.as_slice().cmp(parent)),
            (CallOutputPath::Result(_), CallOutputPath::Store { .. }) => std::cmp::Ordering::Less,
            (CallOutputPath::Store { .. }, CallOutputPath::Result(_)) => {
                std::cmp::Ordering::Greater
            }
        })
        .is_ok();
    if !parent_exists {
        return Err(allocation_error(format!(
            "output {output:?} has no immediate parent row"
        )));
    }
    Ok(())
}

fn validate_parameter_index(
    index: usize,
    parameters: &[PublicCallParameterSummary],
) -> Result<&PublicCallParameterSummary, InfrastructureFailure> {
    parameters.get(index).ok_or_else(|| {
        allocation_error(format!(
            "references parameter index {index}, but the declaration has {} parameter(s)",
            parameters.len()
        ))
    })
}

fn strictly_sorted<T: Ord>(atoms: impl IntoIterator<Item = T>) -> bool {
    atoms.into_iter().is_sorted_by(|left, right| left < right)
}

/// A declaration-side walk can reach a nominal whose provider layout has not been closed in yet.
/// Actual HIR walks must resolve the entire selector path.
pub(crate) enum CallPathType<T> {
    Known(T),
    Unavailable,
}

/// A typed endpoint may be a scalar cell or an allocation. Unavailable provider
/// layouts are not evidence for either classification.
pub(crate) struct CallEndpoint<T> {
    pub ty: T,
    pub allocation: bool,
}

#[allow(dead_code)]
impl CallEndpoint<TypeId> {
    pub(crate) fn hir(ty: TypeId, types: &TypeEnvironment) -> Option<Self> {
        Some(Self {
            ty,
            allocation: classify_hir_type(ty, types)
                .ok()?
                .has_in_place_mutation_surface(),
        })
    }
}

impl CallPathType<TypeId> {
    /// The terminal path must be eligible for tracking as an allocation, not just well-typed.
    pub(crate) fn hir_allocation(ty: TypeId, types: &TypeEnvironment) -> Option<Self> {
        classify_hir_type(ty, types)
            .ok()?
            .has_in_place_mutation_surface()
            .then_some(Self::Known(ty))
    }
}

fn compatible_path_types<T: Eq>(left: &CallPathType<T>, right: &CallPathType<T>) -> bool {
    match (left, right) {
        (CallPathType::Known(left), CallPathType::Known(right)) => left == right,
        _ => true,
    }
}

fn resolved_paths_are_compatible<T: Eq>(
    left: Option<CallPathType<T>>,
    right: Option<CallPathType<T>>,
) -> bool {
    matches!((left, right), (Some(left), Some(right)) if compatible_path_types(&left, &right))
}

/// Validate allocation facts that cross a completed callable boundary using its own layout.
/// `None` success slots denotes Never, as distinct from an ordinary zero-result success lane.
/// Error results, when available, always use slot zero.
pub(crate) fn validate_public_call_allocation_layout<T: Eq>(
    summary: &PublicCallSummary,
    success_slot_count: Option<usize>,
    has_error_slot: bool,
    mut parameter_path: impl FnMut(&CallParameterPath) -> Option<CallPathType<T>>,
    mut result_path: impl FnMut(bool, &CallResultPath) -> Option<CallPathType<T>>,
    mut store_path: impl FnMut(&CallParameterPath, &[CallField]) -> Option<CallPathType<T>>,
) -> Result<(), InfrastructureFailure> {
    validate_return_alias_summary(summary)?;
    match &summary.return_alias {
        FunctionReturnAliasSummary::Pending => {
            return Err(allocation_error(
                "completed callable still has pending facts",
            ));
        }
        FunctionReturnAliasSummary::Known(facts) => {
            for (error_lane, lane, slot_count) in [
                (false, &facts.success, success_slot_count),
                (true, &facts.error, has_error_slot.then_some(1)),
            ] {
                let lane_name = if error_lane { "error" } else { "success" };
                let Some(slot_count) = slot_count else {
                    if lane.can_return() {
                        return Err(allocation_error(format!(
                            "reachable {lane_name} lane has no declared return contract"
                        )));
                    }
                    continue;
                };
                let mut output_type = |output: &CallOutputPath| match output {
                    CallOutputPath::Result(path) if path.slot < slot_count => {
                        result_path(error_lane, path)
                    }
                    CallOutputPath::Result(_) => None,
                    CallOutputPath::Store { target, path } => store_path(target, path),
                };
                for row in lane.sources.iter().chain(&lane.stores) {
                    let output = output_type(&row.output).ok_or_else(|| {
                        allocation_error(format!(
                            "invalid {lane_name} lane output path {:?}",
                            row.output
                        ))
                    })?;
                    if let CallAllocationSource::Parameter(path) = &row.source {
                        let valid = parameter_path(path)
                            .is_some_and(|source| compatible_path_types(&output, &source));
                        if !valid {
                            return Err(allocation_error(format!(
                                "invalid or incompatible {lane_name} lane parameter source {path:?}"
                            )));
                        }
                    }
                }
                for sharing in &lane.sharing {
                    if !resolved_paths_are_compatible(
                        output_type(&sharing.left),
                        output_type(&sharing.right),
                    ) {
                        return Err(allocation_error(format!(
                            "incompatible {lane_name} lane sharing endpoints {sharing:?}"
                        )));
                    }
                    if let CallSharingCondition::Parameters(left, right) = &sharing.condition
                        && !resolved_paths_are_compatible(
                            parameter_path(left),
                            parameter_path(right),
                        )
                    {
                        return Err(allocation_error(format!(
                            "invalid or incompatible {lane_name} lane sharing condition {sharing:?}"
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn allocation_error(message: impl std::fmt::Display) -> InfrastructureFailure {
    InfrastructureFailure::internal(
        CompilerStage::AstConstruction,
        format!("public call summary allocation facts: {message}"),
    )
}
