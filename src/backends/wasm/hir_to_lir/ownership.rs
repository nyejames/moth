//! Ownership-scaffolding lowering helpers.

use crate::backends::wasm::hir_to_lir::context::WasmFunctionLoweringContext;
use crate::backends::wasm::lir::instructions::WasmLirStmt;
use crate::compiler_frontend::analysis::borrow_checker::BorrowDropSiteKind;
use crate::compiler_frontend::hir::ids::BlockId;
use std::collections::BTreeSet;

pub(crate) fn insert_advisory_drops(
    context: &WasmFunctionLoweringContext<'_, '_>,
    block_id: BlockId,
    statements: &mut Vec<WasmLirStmt>,
) {
    // WHAT: project borrow-checker advisory sites into the current `DropIfOwned` scaffolding.
    // WHY: the backend has not migrated to plan-driven cleanup yet. Final full-control release
    // lowering consumes `ValidatedMemoryPlan` and cannot fall back to tracing garbage collection.
    let Some(drop_sites) = context
        .module_context
        .borrow_facts
        .drop_sites_for_block(block_id)
    else {
        return;
    };

    let mut emitted_locals = BTreeSet::new();

    for drop_site in drop_sites {
        if !matches!(
            drop_site.kind,
            BorrowDropSiteKind::BlockExit | BorrowDropSiteKind::Return | BorrowDropSiteKind::Break
        ) {
            continue;
        }

        for local_id in &drop_site.locals {
            let Some(lir_local_id) = context.local_map.get(local_id).copied() else {
                continue;
            };
            if !context.is_handle_local(lir_local_id) {
                continue;
            }
            if !emitted_locals.insert(lir_local_id.0) {
                continue;
            }

            statements.push(WasmLirStmt::DropIfOwned {
                value: lir_local_id,
            });
        }
    }
}
