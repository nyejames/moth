//! Call-effect summary translation.
//!
//! WHAT: translates retained and external call summaries into normalized argument
//!       access and result provenance.
//! WHY: keeping call-boundary projection separate leaves the problem builder focused on HIR
//!      traversal and event emission.

use super::super::events::AccessKind;
use super::super::origins::{CallResultProvenance, CallResultUnknownReason};
use super::{FunctionProblemBuilder, compiler_error};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalAccessKind, ExternalReturnAlias,
};
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallParameterAccess,
};

impl<'a> FunctionProblemBuilder<'a> {
    pub(super) fn call_effect(
        &self,
        target: &CallTarget,
        argument_count: usize,
    ) -> Result<(Vec<AccessKind>, CallResultProvenance), CompilerError> {
        if let CallTarget::External(id) = target {
            let Some(registry) = self.external_registry else {
                return Ok((
                    vec![AccessKind::Exclusive; argument_count],
                    CallResultProvenance::Unknown(CallResultUnknownReason::OpaqueExternal),
                ));
            };
            let Some(definition) = registry.get_function_by_id(*id) else {
                return Err(compiler_error(format!(
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
                return Err(compiler_error(format!(
                    "Boracle external call {:?} has {} arguments but metadata has {} parameters",
                    id,
                    argument_count,
                    accesses.len()
                )));
            }
            let provenance = match definition.hir_return_alias() {
                ExternalReturnAlias::Fresh => CallResultProvenance::Fresh,
                ExternalReturnAlias::AliasArgs(indices) => CallResultProvenance::AliasParams(
                    indices.into_iter().collect::<Vec<_>>().into_boxed_slice(),
                ),
            };
            return Ok((accesses, provenance));
        }

        let summary = match target {
            CallTarget::Local(id) => self.local_summaries.and_then(|summaries| summaries.get(id)),
            CallTarget::CrossModule(id) => self.module.imported_call_summaries.get(id),
            CallTarget::ModulePrivate(id) => self.module.module_private_call_summaries.get(id),
            CallTarget::Generated(id) => self.module.generated_call_summaries.get(id),
            CallTarget::External(_) => None,
        };
        let Some(summary) = summary else {
            if let CallTarget::CrossModule(id) = target {
                return Err(compiler_error(format!(
                    "Boracle problem extraction is missing the provider call summary for imported function {id:?}"
                )));
            }
            if let CallTarget::Local(function_id) = target
                && let Some(accesses) =
                    self.local_parameter_accesses(*function_id, argument_count)?
            {
                return Ok((
                    accesses,
                    CallResultProvenance::Unknown(CallResultUnknownReason::MissingSummary),
                ));
            }
            return Ok((
                // Missing call metadata must not under-approximate a mutable callee. The
                // reference path prefers conservative false positives over proving that an
                // unresolved boundary is shared.
                vec![AccessKind::Exclusive; argument_count],
                CallResultProvenance::Unknown(CallResultUnknownReason::MissingSummary),
            ));
        };
        if summary.parameters.len() != argument_count {
            return Err(compiler_error(format!(
                "Boracle call summary for {target:?} has {} parameters but call has {argument_count} arguments",
                summary.parameters.len()
            )));
        }
        let accesses = summary
            .parameters
            .iter()
            .map(|parameter| match parameter.access {
                PublicCallParameterAccess::Mutable => AccessKind::Exclusive,
                PublicCallParameterAccess::Shared | PublicCallParameterAccess::Reactive => {
                    AccessKind::Shared
                }
            })
            .collect();
        let provenance = match &summary.return_alias {
            FunctionReturnAliasSummary::Fresh => CallResultProvenance::Fresh,
            FunctionReturnAliasSummary::AliasParams(indices) => {
                CallResultProvenance::AliasParams(indices.iter().copied().collect())
            }
            FunctionReturnAliasSummary::Unknown => {
                CallResultProvenance::Unknown(CallResultUnknownReason::SummaryUnknown)
            }
        };
        Ok((accesses, provenance))
    }
}
