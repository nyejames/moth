//! Output reservation estimates for the TIR fold reducer.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::tir::ids::TemplateIrNodeId;
use crate::compiler_frontend::ast::templates::tir::node::TemplateIrNodeKind;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::symbols::string_interning::StringTable;

const FOLD_LOOP_RESERVE_BYTE_CAP: usize = 64 * 1024;
const FOLD_RANGE_LOOP_RESERVE_ITERATION_CAP: usize = 256;

#[derive(Clone, Copy)]
pub(super) enum FoldEstimateMode {
    Structural,
    Aggregate { output_len: usize },
}

pub(super) fn reserve_tir_fold_output_buffer(estimated_bytes: usize) -> String {
    add_ast_counter(
        AstCounter::TemplateEstimatedFoldOutputBytes,
        estimated_bytes,
    );
    String::with_capacity(estimated_bytes)
}

pub(super) fn record_tir_fold_output_estimate_miss(actual_len: usize, estimated_bytes: usize) {
    if actual_len > estimated_bytes {
        add_ast_counter(
            AstCounter::TemplateFoldOutputEstimateMissBytes,
            actual_len - estimated_bytes,
        );
    }
}

pub(super) fn estimate_loop_aggregate_bytes(body_estimate: usize, iteration_count: usize) -> usize {
    body_estimate
        .saturating_mul(iteration_count)
        .min(FOLD_LOOP_RESERVE_BYTE_CAP)
}

pub(super) fn estimated_range_iteration_count(loop_limit: usize) -> usize {
    std::cmp::min(loop_limit, FOLD_RANGE_LOOP_RESERVE_ITERATION_CAP)
}

pub(super) fn record_tir_fold_output_intern(byte_len: usize) {
    add_ast_counter(AstCounter::TirFoldStringInternCalls, 1);
    add_ast_counter(AstCounter::TirFoldOutputBytes, byte_len);
    add_ast_counter(AstCounter::TemplateFoldStringInternCalls, 1);
    add_ast_counter(AstCounter::TemplateFoldOutputBytes, byte_len);
}

pub(super) fn estimate_tir_node_output_bytes(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
    string_table: &StringTable,
    mode: FoldEstimateMode,
) -> Result<usize, TemplateError> {
    let node = store
        .get_node(node_id)
        .ok_or_else(|| super::reducer::missing_node_diagnostic(node_id))?;

    match &node.kind {
        TemplateIrNodeKind::Text { text, .. } => Ok(string_table.resolve(*text).len()),
        TemplateIrNodeKind::Sequence { children } => children
            .iter()
            .map(|child| estimate_tir_node_output_bytes(store, *child, string_table, mode))
            .sum(),
        TemplateIrNodeKind::AggregateOutput => match mode {
            FoldEstimateMode::Structural => Ok(0),
            FoldEstimateMode::Aggregate { output_len } => Ok(output_len),
        },
        TemplateIrNodeKind::ChildTemplate { .. }
        | TemplateIrNodeKind::DynamicExpression { .. } => Ok(0),
        TemplateIrNodeKind::Slot { .. } => Ok(0),
        TemplateIrNodeKind::BranchChain { .. }
        | TemplateIrNodeKind::Loop { .. }
        | TemplateIrNodeKind::InsertContribution { .. }
        | TemplateIrNodeKind::LoopControl { .. } => match mode {
            FoldEstimateMode::Structural => Ok(0),
            FoldEstimateMode::Aggregate { .. } => Err(CompilerError::compiler_error(
                "TIR fold: malformed aggregate wrapper subtree contains a node kind that cannot be estimated inside a wrapper.",
            )
            .into()),
        },
        TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => Err(
            CompilerError::compiler_error(
                "TIR fold: runtime slot nodes cannot enter constant-fold output estimation.",
            )
            .into(),
        ),
    }
}
