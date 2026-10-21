//! Prepared diagnostics that cross the premerge boundary without a legacy vessel.
//!
//! WHAT: owns family drafts with captured type-display snapshots, the batch-local string table,
//! explicit path rows, and frozen-identity handles collected during Pass A.
//! WHY: a premerge batch joins the run accumulator without constructing `CompilerMessages` or
//! retaining its live type environment.

use super::compiler_errors::{RenderPathContext, RenderTypeContext};
use super::diagnostic_bag::PremergeDiagnosticBatch;
use super::diagnostics::PreparedDiagnostic;
use super::diagnostics::legacy_adapter::prepare_diagnostic;
use crate::compiler_frontend::source::FrozenIdentityHandle;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Batch-local string, path, and frozen-identity ownership.
pub(super) struct PreparedBatchIdentity {
    pub(super) string_table: StringTable,
    pub(super) path_rows: Vec<RenderPathContext>,
    pub(super) required_handles: Vec<FrozenIdentityHandle>,
}

/// Move-only premerge diagnostics ready to append directly to a boundary run.
///
/// The boxed identity side store keeps this carrier at most 48 bytes. Each draft owns its captured
/// type snapshot so append remaps the pair in place without a second conversion.
#[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S5b carrier cut"))]
pub(crate) struct PreparedDiagnosticBatch {
    pub(super) prepared: Vec<PreparedDiagnostic>,
    pub(super) identity: Box<PreparedBatchIdentity>,
}

const _: () = assert!(std::mem::size_of::<PreparedDiagnosticBatch>() <= 48);

impl PreparedDiagnosticBatch {
    /// Capture a premerge batch against its own retained contexts using the legacy adapter's Pass A.
    #[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S5b carrier cut"))]
    pub(crate) fn from_premerge(batch: PremergeDiagnosticBatch) -> Self {
        let (bag, string_table, type_rows, path_rows) = batch.into_parts();
        let diagnostics = bag.into_diagnostics();
        let mut required_handles = Vec::new();
        let mut prepared = Vec::with_capacity(diagnostics.len());

        for (index, diagnostic) in diagnostics.iter().enumerate() {
            let explicit_path = path_rows
                .iter()
                .position(|row| row.diagnostic_range.contains(&index));
            let environment = explicit_path
                .and_then(|_| {
                    type_rows
                        .iter()
                        .find(|row| row.diagnostic_range.contains(&index))
                })
                .map(|row: &RenderTypeContext| &row.type_environment);
            let mut item = prepare_diagnostic(diagnostic, environment, &mut required_handles);
            item.explicit_path = explicit_path;
            prepared.push(item);
        }

        Self {
            prepared,
            identity: Box::new(PreparedBatchIdentity {
                string_table,
                path_rows,
                required_handles,
            }),
        }
    }
}
