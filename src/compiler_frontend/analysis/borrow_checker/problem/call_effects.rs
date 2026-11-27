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
use crate::compiler_frontend::hir::expressions::ValueKind;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::hir_datatypes::classify_hir_type;
use crate::compiler_frontend::hir::ids::HirValueId;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::hir::utils::collect_structural_blocks;
use crate::compiler_frontend::public_call_summary::{
    CallAllocationSource, CallField, CallLaneSummary, CallOutputPath, FunctionReturnAliasSummary,
    PublicCallParameterAccess,
};

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
                let opaque_stores = self.may_have_opaque_stores(args, &accesses, local, false)?;
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
                let lane = if success {
                    &facts.success
                } else {
                    &facts.error
                };
                let mut provenance = lane_root_provenance(lane);
                // Exact scalar lanes do not need allocation rows. The destination's
                // validated type, not the absence of rows alone, proves a fresh value.
                if lane.reachable
                    && lane.sources.is_empty()
                    && lane.sharing.is_empty()
                    && let [destination] = results
                    && let Some(types) = self.type_environment
                    && let Some(local) = self
                        .module
                        .blocks
                        .iter()
                        .flat_map(|block| &block.locals)
                        .find(|local| local.id == destination.local())
                    && !classify_hir_type(local.ty, types)?.has_in_place_mutation_surface()
                {
                    provenance = CallResultProvenance::Fresh;
                }
                let opaque_stores = !lane.stores.is_empty()
                    || self.may_have_opaque_stores(args, &accesses, local, lane.reachable)?;
                (provenance, opaque_stores)
            }
            FunctionReturnAliasSummary::Pending => (
                CallResultProvenance::Unknown(CallResultUnknownReason::MissingSummary),
                self.may_have_opaque_stores(args, &accesses, local, false)?,
            ),
        };
        Ok((accesses, provenance, opaque_stores))
    }

    fn may_have_opaque_stores(
        &self,
        args: &[HirValueId],
        accesses: &[AccessKind],
        local: Option<&HirFunction>,
        has_allocation_store_facts: bool,
    ) -> Result<bool, InfrastructureFailure> {
        let mut needs_cell_contract = false;
        for (&value, &access) in args.iter().zip(accesses) {
            if access != AccessKind::Exclusive {
                continue;
            }
            let Some(types) = self.type_environment else {
                return Ok(true);
            };
            let ty = self.module.expressions.expression(value).ty;
            if classify_hir_type(ty, types)?.has_in_place_mutation_surface() {
                if !has_allocation_store_facts {
                    return Ok(true);
                }
            } else {
                needs_cell_contract = true;
            }
        }
        Ok(needs_cell_contract
            && !local.is_some_and(|function| self.proves_no_cell_transition(function)))
    }

    /// The scalar payload can change without changing the storage graph. A whole-place
    /// acquisition or an unmodelled delegated call doesn't have that guarantee.
    fn proves_no_cell_transition(&self, function: &HirFunction) -> bool {
        collect_structural_blocks(self.module, function.entry)
            .into_iter()
            .all(|id| {
                let Some(block) = self.module.block(id) else {
                    return false;
                };
                if matches!(
                    block.terminator,
                    HirTerminator::Invoke {
                        operation: HirFallibleOperation::Call { .. },
                        ..
                    } | HirTerminator::NeverCall { .. }
                ) {
                    return false;
                }
                block
                    .statements
                    .iter()
                    .all(|statement| match statement.kind {
                        HirStatementKind::Call { .. } => false,
                        HirStatementKind::Write {
                            target: target @ HirWriteTarget::AssignPlace(place),
                            value,
                        } if place.projections.is_empty() => {
                            let value = self.module.expressions.expression(value);
                            value.value_kind != ValueKind::Place
                                || target.is_direct_self_update_of(value)
                        }
                        _ => true,
                    })
            })
    }
}

/// The reference event can carry one root origin, not a correlated graph of result paths.
fn lane_root_provenance(lane: &CallLaneSummary) -> CallResultProvenance {
    let unknown = CallResultProvenance::Unknown(CallResultUnknownReason::SummaryUnknown);
    if !lane.reachable || lane.sources.is_empty() || !lane.sharing.is_empty() {
        return unknown;
    }
    let mut fresh = false;
    let mut indices = Vec::new();
    for row in &lane.sources {
        let CallOutputPath::Result(path) = &row.output else {
            return unknown;
        };
        if path.slot != 0 {
            return unknown;
        }
        if !path.path.is_empty() {
            if row.source != CallAllocationSource::Fresh
                || !path
                    .path
                    .iter()
                    .all(|field| matches!(field, CallField::Field(_)))
            {
                return unknown;
            }
            continue;
        }
        match &row.source {
            CallAllocationSource::Parameter(path) if path.path.is_empty() => {
                indices.push(path.index);
            }
            CallAllocationSource::Fresh => fresh = true,
            _ => return unknown,
        }
    }
    if indices.is_empty() && fresh {
        CallResultProvenance::Fresh
    } else if indices.is_empty() || fresh || lane.sources.len() != indices.len() {
        unknown
    } else {
        indices.sort_unstable();
        indices.dedup();
        CallResultProvenance::AliasParams(indices.into_boxed_slice())
    }
}
