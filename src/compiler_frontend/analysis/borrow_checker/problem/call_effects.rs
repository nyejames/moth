//! Call-effect summary translation.
//!
//! WHAT: translates retained and external call summaries into normalized argument
//!       access and result provenance.
//! WHY: keeping call-boundary projection separate leaves the problem builder focused on HIR
//!      traversal and event emission.

use super::super::events::AccessKind;
use super::super::origins::{CallResultProvenance, CallResultUnknownReason};
use super::{FunctionProblemBuilder, internal_failure};
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalAccessKind, ExternalReturnAlias,
};
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::hir_datatypes::classify_hir_type;
use crate::compiler_frontend::hir::ids::HirValueId;
use crate::compiler_frontend::hir::statements::HirLocalDestination;
#[cfg(test)]
use crate::compiler_frontend::public_call_summary::CallField;
use crate::compiler_frontend::public_call_summary::typed::{
    CellPort, Coverage, EntryPort, ExportPort, Footprint, IdentityKind, Lane, ObjectRef,
    OutputPort, PortId, Predicate, ScalarSource, TypedCallAllocationFacts, ValueSource,
};
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallParameterAccess,
};

pub(super) fn typed_lane_has_opaque_stores(
    lane: &Lane,
    observe: impl FnMut(Predicate) -> Result<Option<bool>, InfrastructureFailure>,
    mut represents_cell: impl FnMut(&CellPort) -> bool,
) -> Result<bool, InfrastructureFailure> {
    let caller_cell = |cell: &CellPort| match cell {
        CellPort::Terminal(_) => true,
        CellPort::Slot(wrapper) | CellPort::Edge(wrapper, _) => {
            matches!(wrapper, ObjectRef::Entry(_))
        }
    };
    let mut refused = false;
    lane.visit_compatible(observe, |_, product| {
        refused |= product
            .components()
            .iter()
            .flat_map(|component| &component.alternatives)
            .any(|post| {
                post.bindings
                    .iter()
                    .any(|(wrapper, _)| matches!(wrapper, ObjectRef::Entry(_)))
                    || post
                        .cells
                        .iter()
                        .any(|(cell, _)| caller_cell(cell) && !represents_cell(cell))
                    || post
                        .families
                        .iter()
                        .any(|(family, _)| matches!(family.receiver, ObjectRef::Entry(_)))
                    || match &post.coverage {
                        Coverage::Complete => false,
                        Coverage::Unresolved(Footprint::WritableReach(_)) => true,
                        Coverage::Unresolved(Footprint::Locations(locations)) => {
                            locations.iter().any(|location| match location {
                                OutputPort::Result(_) => false,
                                OutputPort::Binding(wrapper) => {
                                    matches!(wrapper, ObjectRef::Entry(_))
                                }
                                OutputPort::Cell(cell) => caller_cell(cell),
                                OutputPort::Family(family) => {
                                    matches!(family.receiver, ObjectRef::Entry(_))
                                }
                            })
                        }
                    }
            });
        Ok(())
    })?;
    Ok(refused)
}

#[cfg(test)]
#[test]
fn typed_family_store_refusal_is_limited_to_selected_guard_and_outcome() {
    use crate::compiler_frontend::public_call_summary::typed::{
        Component, FamilyPort, FamilyPost, Guard, PortId, Post, Product, SupportId,
        TypedCallAllocationFacts,
    };
    let receiver = PortId(0);
    let writable = Predicate::Writable(receiver);
    let product = Product::new(vec![Component {
        support: SupportId(0),
        alternatives: vec![Post {
            families: vec![(
                FamilyPort {
                    receiver: ObjectRef::Entry(receiver),
                    selector: CallField::Elem,
                },
                FamilyPost::ReplaceAll(Vec::new()),
            )],
            ..Default::default()
        }],
    }])
    .unwrap();
    let returning = Lane::Returning(Product::default());
    let facts = TypedCallAllocationFacts {
        success: Lane::guarded(Guard::new([(writable, true)]).unwrap(), product).union(&returning),
        error: returning,
        ..Default::default()
    };
    for (error, writable, expected) in [
        (false, Some(true), true),
        (false, Some(false), false),
        (false, None, true),
        (true, Some(true), false),
    ] {
        assert_eq!(
            typed_lane_has_opaque_stores(facts.lane(error), |_| Ok(writable), |_| true).unwrap(),
            expected
        );
    }
}

impl<'a> FunctionProblemBuilder<'a> {
    pub(super) fn call_effect(
        &self,
        target: &CallTarget,
        args: &[HirValueId],
        results: &[HirLocalDestination],
        success: bool,
    ) -> Result<(Vec<AccessKind>, CallResultProvenance, bool), InfrastructureFailure> {
        let argument_count = args.len();
        if let CallTarget::External(id) = target {
            let Some(registry) = self.external_registry else {
                return Ok((
                    vec![AccessKind::Exclusive; argument_count],
                    CallResultProvenance::Unknown(CallResultUnknownReason::OpaqueExternal),
                    argument_count != 0,
                ));
            };
            let Some(definition) = registry.get_function_by_id(*id) else {
                return Err(internal_failure(format!(
                    "Boracle problem extraction cannot resolve external call {:?}",
                    id
                )));
            };
            let accesses = definition
                .parameters
                .iter()
                .map(|parameter| match parameter.access_kind {
                    ExternalAccessKind::Shared => AccessKind::Shared,
                    ExternalAccessKind::Mutable => AccessKind::Exclusive,
                })
                .collect::<Vec<_>>();
            if accesses.len() != argument_count {
                return Err(internal_failure(format!(
                    "Boracle external call {:?} has {} arguments but metadata has {} parameters",
                    id,
                    argument_count,
                    accesses.len()
                )));
            }
            let unknown = CallResultProvenance::Unknown(CallResultUnknownReason::OpaqueExternal);
            let provenance = match (success, definition.hir_return_sources()) {
                (true, [slot]) => match &slot.alias {
                    ExternalReturnAlias::Fresh => CallResultProvenance::Fresh,
                    ExternalReturnAlias::AliasArgs(indices) => {
                        CallResultProvenance::AliasParams(indices.as_slice().into())
                    }
                },
                // A single origin cannot model independent or correlated native slots.
                _ => unknown,
            };
            let opaque_stores = match CollectionBuiltinOp::from_external_function_id(*id) {
                Some(operation) => success && operation.requires_mutable_receiver(),
                None => accesses.contains(&AccessKind::Exclusive),
            };
            return Ok((accesses, provenance, opaque_stores));
        }

        let local = match target {
            CallTarget::Local(id) => self
                .module
                .functions
                .iter()
                .find(|function| function.id == *id),
            _ => None,
        };
        let summary = match target {
            CallTarget::Local(id) => self.local_summaries.and_then(|summaries| summaries.get(id)),
            CallTarget::CrossModule(id) => self.module.imported_call_summaries.get(id),
            CallTarget::ModulePrivate(id) => self.module.module_private_call_summaries.get(id),
            CallTarget::Generated(id) => self.module.generated_call_summaries.get(id.as_ref()),
            CallTarget::External(_) => None,
        };
        let Some(summary) = summary else {
            if let CallTarget::CrossModule(id) = target {
                return Err(internal_failure(format!(
                    "Boracle problem extraction is missing the provider call summary for imported function {id:?}"
                )));
            }
            if let Some(function) = local {
                let accesses = self.local_parameter_accesses(function, argument_count)?;
                let opaque_stores = accesses.contains(&AccessKind::Exclusive);
                return Ok((
                    accesses,
                    CallResultProvenance::Unknown(CallResultUnknownReason::MissingSummary),
                    opaque_stores,
                ));
            }
            return Ok((
                // Missing call metadata must not under-approximate a mutable callee. The
                // reference path prefers conservative false positives over proving that an
                // unresolved boundary is shared.
                vec![AccessKind::Exclusive; argument_count],
                CallResultProvenance::Unknown(CallResultUnknownReason::MissingSummary),
                argument_count != 0,
            ));
        };
        if summary.parameters.len() != argument_count {
            return Err(internal_failure(format!(
                "Boracle call summary for {target:?} has {} parameters but call has {argument_count} arguments",
                summary.parameters.len()
            )));
        }
        let accesses = summary
            .parameters
            .iter()
            .map(|parameter| match parameter.access {
                PublicCallParameterAccess::Mutable => AccessKind::Exclusive,
                PublicCallParameterAccess::Shared => AccessKind::Shared,
            })
            .collect::<Vec<_>>();
        let (provenance, opaque_stores) = match &summary.return_alias {
            FunctionReturnAliasSummary::Known(facts) => {
                let lane = facts.lane(!success);
                let parameter = |port: PortId| port.parameter(&facts.ports);
                let observe = |predicate| {
                    Ok(match predicate {
                        Predicate::Writable(port) => {
                            (accesses[parameter(port)] == AccessKind::Shared).then_some(false)
                        }
                        Predicate::Equal(IdentityKind::Wrapper, left, right) => {
                            let a = self.module.expressions.expression(args[parameter(left)]);
                            let b = self.module.expressions.expression(args[parameter(right)]);
                            match (&a.kind, &b.kind) {
                                (HirExpressionKind::Load(a), HirExpressionKind::Load(b))
                                    if a.root == b.root
                                        && a.projections.is_empty()
                                        && b.projections.is_empty() =>
                                {
                                    Some(true)
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    })
                };
                let provenance = if results.len() == 1 {
                    lane_root_provenance(facts, lane, observe)?
                } else {
                    CallResultProvenance::Unknown(CallResultUnknownReason::SummaryUnknown)
                };
                let opaque_stores = typed_lane_has_opaque_stores(lane, observe, |cell| {
                    let (CellPort::Terminal(port) | CellPort::Slot(ObjectRef::Entry(port))) = cell
                    else {
                        return false;
                    };
                    self.type_environment.is_some_and(|types| {
                        classify_hir_type(
                            self.module
                                .expressions
                                .expression(args[parameter(*port)])
                                .ty,
                            types,
                        )
                        .is_ok_and(|shape| !shape.has_in_place_mutation_surface())
                    })
                })?;
                (provenance, opaque_stores)
            }
            FunctionReturnAliasSummary::Pending => (
                CallResultProvenance::Unknown(CallResultUnknownReason::MissingSummary),
                accesses.contains(&AccessKind::Exclusive),
            ),
        };
        Ok((accesses, provenance, opaque_stores))
    }
}

/// The reference event can carry one root origin, not a correlated graph.
fn lane_root_provenance(
    facts: &TypedCallAllocationFacts,
    lane: &Lane,
    observe: impl FnMut(Predicate) -> Result<Option<bool>, InfrastructureFailure>,
) -> Result<CallResultProvenance, InfrastructureFailure> {
    let mut fresh = false;
    let mut unknown = false;
    let mut indices = Vec::new();
    lane.visit_compatible(observe, |_, product| {
        for post in product
            .components()
            .iter()
            .flat_map(|component| &component.alternatives)
        {
            if !matches!(post.coverage, Coverage::Complete) {
                unknown = true;
                continue;
            }
            for (slot, value) in &post.results {
                if *slot != 0 || !value.dependencies.is_empty() {
                    unknown = true;
                    continue;
                }
                match &value.source {
                    ValueSource::Scalar(ScalarSource::Independent) => fresh = true,
                    ValueSource::Allocation(ObjectRef::Exact(ExportPort::Result(path)))
                        if path.slot == 0 && path.path.is_empty() =>
                    {
                        fresh = true
                    }
                    ValueSource::Allocation(ObjectRef::Entry(port))
                    | ValueSource::Scalar(ScalarSource::EntryValue(port)) => {
                        match &facts.ports[port.0 as usize] {
                            EntryPort::Value { parameter, path } if path.is_empty() => {
                                indices.push(*parameter)
                            }
                            _ => unknown = true,
                        }
                    }
                    _ => unknown = true,
                }
            }
        }
        Ok(())
    })?;
    Ok(if !unknown && fresh && indices.is_empty() {
        CallResultProvenance::Fresh
    } else if !unknown && !fresh && !indices.is_empty() {
        indices.sort_unstable();
        indices.dedup();
        CallResultProvenance::AliasParams(indices.into_boxed_slice())
    } else {
        CallResultProvenance::Unknown(CallResultUnknownReason::SummaryUnknown)
    })
}
