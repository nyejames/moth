//! Expression storage census at the last-reader publication boundary.
//!
//! Counts physical rows and typed side buffers once, including ordinary dead rows
//! retained until reachable compaction. Nested statement, string and fact allocations
//! are excluded from these inline-buffer measurements, not reported as zero.

use std::mem::{align_of, size_of};

use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionOp, ExpressionRow,
};
use crate::compiler_frontend::instrumentation::ast_counters::record_expression_storage_census;
use crate::compiler_frontend::instrumentation::{
    AstCounter, increment_ast_counter, record_ast_counter_max, with_expression_census_counters,
};

/// Records the typed expression store's storage shape at the last-reader publication boundary.
///
/// WHAT: observes the issuing store, counting physical rows and typed side buffers.
/// WHY: the census measures the store about to be frozen, not an owning expression tree.
pub(super) fn record_published_expression_census(expression_builder: &AstExpressionBuilder) {
    with_expression_census_counters(|| {
        increment_ast_counter(AstCounter::CensusModules);
        record_expression_storage_census(expression_builder.storage_statistics());
        record_ast_counter_max(
            AstCounter::CensusExpressionRowSizeMax,
            size_of::<ExpressionRow>(),
        );
        record_ast_counter_max(
            AstCounter::CensusExpressionRowAlignMax,
            align_of::<ExpressionRow>(),
        );
        record_ast_counter_max(
            AstCounter::CensusExpressionOpSizeMax,
            size_of::<ExpressionOp>(),
        );
        record_ast_counter_max(
            AstCounter::CensusExpressionOpAlignMax,
            align_of::<ExpressionOp>(),
        );
    });
}
