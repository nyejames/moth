//! Borrow-checker transfer driver.
//!
//! WHAT: coordinates one-block transfer for the fixed-point borrow analysis.
//! Statement/terminator rules live in submodules so external call semantics,
//! access validation, and fact buffering stay inspectable.
//! WHY: transfer owns borrow-rule effects and side-table fact emission; it must
//! not mutate HIR or perform backend ownership lowering.

mod access;
mod call_semantics;
mod facts;

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::diagnostics::BorrowDiagnostics;
use crate::compiler_frontend::analysis::borrow_checker::state::{BorrowState, FunctionLayout};
use crate::compiler_frontend::analysis::borrow_checker::types::{
    BorrowStateSnapshot, ReactiveInvalidationFact, StatementBorrowFact, TerminatorBorrowFact,
    ValueBorrowFact,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, HirValueId};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use rustc_hash::FxHashMap;

use access::{
    AggregateTransferContext, transfer_aggregate_expression_ownership, transfer_statement,
    transfer_terminator,
};
use facts::ValueFactBuffer;

pub(super) struct BorrowTransferContext<'a> {
    // WHAT: shared lookup/diagnostic tables for one function transfer pass.
    // WHY: avoids repeated module scans while statements/terminators are analyzed.
    pub external_package_registry: &'a ExternalPackageRegistry,
    pub public_call_summaries: &'a FxHashMap<FunctionId, PublicCallSummary>,
    pub diagnostics: BorrowDiagnostics<'a>,
}

/// Accumulated statistics and emitted facts for one block transfer.
#[derive(Debug, Default)]
pub(super) struct BlockTransferStats {
    // Counters.
    pub statements_analyzed: usize,
    pub terminators_analyzed: usize,
    pub conflicts_checked: usize,
    pub mutable_call_sites: usize,

    // Per-statement entry snapshots.
    pub statement_entry_states: Vec<(HirNodeId, BorrowStateSnapshot)>,

    // Emitted borrow facts.
    pub statement_facts: Vec<(HirNodeId, StatementBorrowFact)>,
    pub terminator_fact: Option<(BlockId, TerminatorBorrowFact)>,
    pub value_facts: Vec<(HirValueId, ValueBorrowFact)>,
    pub reactive_invalidations: Vec<(HirNodeId, Vec<ReactiveInvalidationFact>)>,
}

/// Executes one forward transfer step for a single basic block.
///
/// The caller owns fixed-point scheduling. This function only applies local
/// transfer rules and emits borrow facts for the processed block.
pub(super) fn transfer_block(
    context: &BorrowTransferContext<'_>,
    layout: &FunctionLayout,
    block: &HirBlock,
    state: &mut BorrowState,
) -> Result<BlockTransferStats, BorrowCheckError> {
    let mut stats = BlockTransferStats::default();
    let mut value_fact_buffer = ValueFactBuffer::new(layout.local_count());

    for statement in &block.statements {
        stats
            .statement_entry_states
            .push((statement.id, state.to_snapshot(&layout.local_ids)));
        transfer_statement(
            context,
            layout,
            state,
            block.id,
            statement,
            &mut stats,
            &mut value_fact_buffer,
        )?;
        stats.statements_analyzed += 1;
    }

    transfer_terminator(
        context,
        layout,
        state,
        block.id,
        &block.terminator,
        &mut stats,
        &mut value_fact_buffer,
    )?;
    stats.terminators_analyzed += 1;

    // Aggregate literal children in return terminators receive optional transfer analysis.
    let mut aggregate_context = AggregateTransferContext {
        diagnostics: &context.diagnostics,
        value_fact_buffer: &mut value_fact_buffer,
    };

    match &block.terminator {
        HirTerminator::Return(value)
        | HirTerminator::ReturnSuccess(value)
        | HirTerminator::ReturnError(value) => {
            let terminator_order = layout.terminator_order_or_unknown(block.id);
            let location = context
                .diagnostics
                .terminator_error_location(block.id, &block.terminator);
            transfer_aggregate_expression_ownership(
                layout,
                state,
                value,
                block.id,
                terminator_order,
                location,
                &mut aggregate_context,
            )?;
        }
        HirTerminator::FallibleBranch { result, .. } => {
            let terminator_order = layout.terminator_order_or_unknown(block.id);
            let location = context
                .diagnostics
                .terminator_error_location(block.id, &block.terminator);
            transfer_aggregate_expression_ownership(
                layout,
                state,
                result,
                block.id,
                terminator_order,
                location,
                &mut aggregate_context,
            )?;
        }
        _ => {}
    }

    stats.value_facts = value_fact_buffer.into_serialized(layout);
    Ok(stats)
}
