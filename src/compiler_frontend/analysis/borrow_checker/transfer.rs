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
use crate::compiler_frontend::analysis::borrow_checker::state::{
    BorrowState, FunctionLayout, LocalState,
};
#[cfg(any(test, feature = "show_borrow_checker"))]
use crate::compiler_frontend::analysis::borrow_checker::types::BorrowStateSnapshot;
use crate::compiler_frontend::analysis::borrow_checker::types::{
    StatementBorrowFact, TerminatorBorrowFact, ValueBorrowFact,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, HirValueId, LocalId};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::semantic_identity::{
    GeneratedFunctionIdentity, ModulePrivateExecutableIdentity, OriginFunctionId,
};
use rustc_hash::FxHashMap;

use access::{
    AggregateTransferContext, transfer_aggregate_expression_ownership, transfer_statement,
    transfer_terminator,
};
use facts::ValueFactBuffer;

#[cfg(test)]
#[path = "transfer/tests/value_fact_buffer_tests.rs"]
mod value_fact_buffer_tests;

pub(super) struct BorrowTransferContext<'a> {
    // WHAT: shared lookup/diagnostic tables for one function transfer pass.
    // WHY: avoids repeated module scans while statements/terminators are analyzed.
    pub external_package_registry: &'a ExternalPackageRegistry,
    pub public_call_summaries: &'a FxHashMap<FunctionId, PublicCallSummary>,
    pub imported_call_summaries: &'a FxHashMap<OriginFunctionId, PublicCallSummary>,
    pub module_private_call_summaries:
        &'a FxHashMap<ModulePrivateExecutableIdentity, PublicCallSummary>,
    pub generated_call_summaries: &'a FxHashMap<GeneratedFunctionIdentity, PublicCallSummary>,
    pub expressions: &'a HirExpressionStore,
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

    // Per-statement snapshots are diagnostic presentation data, not borrow facts.
    #[cfg(any(test, feature = "show_borrow_checker"))]
    pub statement_entry_states: Vec<(HirNodeId, BorrowStateSnapshot)>,

    // Emitted borrow facts.
    pub statement_facts: Vec<(HirNodeId, StatementBorrowFact)>,
    pub terminator_fact: Option<(BlockId, TerminatorBorrowFact)>,
    pub value_facts: Vec<(HirValueId, ValueBorrowFact)>,
    // Captured operation payloads are installed only after the outgoing edge kills old outcomes.
    pub invoke_results: Vec<(LocalId, LocalState)>,
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
        #[cfg(any(test, feature = "show_borrow_checker"))]
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
        value_fact_buffer.finish_use();
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
        expressions: context.expressions,
        value_fact_buffer: &mut value_fact_buffer,
    };

    match &block.terminator {
        HirTerminator::Return(values) => {
            let terminator_order = layout.terminator_order_or_unknown(block.id);
            let location = context.diagnostics.terminator_error_span(block.id);
            for value in context.expressions.values(*values) {
                transfer_aggregate_expression_ownership(
                    layout,
                    state,
                    *value,
                    block.id,
                    terminator_order,
                    location,
                    &mut aggregate_context,
                )?;
            }
        }
        HirTerminator::ReturnError(value) => {
            let terminator_order = layout.terminator_order_or_unknown(block.id);
            let location = context.diagnostics.terminator_error_span(block.id);
            transfer_aggregate_expression_ownership(
                layout,
                state,
                *value,
                block.id,
                terminator_order,
                location,
                &mut aggregate_context,
            )?;
        }
        HirTerminator::Invoke { operation, .. } => {
            let terminator_order = layout.terminator_order_or_unknown(block.id);
            let location = context.diagnostics.terminator_error_span(block.id);
            for value in operation.operands(context.expressions) {
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
        }
        _ => {}
    }

    value_fact_buffer.finish_use();
    stats.value_facts = value_fact_buffer.into_serialized(layout);
    Ok(stats)
}
