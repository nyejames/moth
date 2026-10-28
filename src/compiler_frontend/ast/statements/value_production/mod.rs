//! Shared value-production infrastructure for local value-producing blocks.
//!
//! WHAT: owns `then` value parsing, active value targets, all-path exit summaries
//! and reachable produced-value traversal shared by catch handlers and value-producing
//! control flow.
//! WHY: fallback parsing and terminal-only enforcement are shared by catch and value
//!      blocks, so value-block syntax does not duplicate the same arity, coercion and
//!      completeness logic.
//!
//! This module owns local value production only. It does not own:
//! - result propagation (`!` and `return!`),
//! - option type construction,
//! - tuple user syntax,
//! - template control flow.

pub(crate) mod completeness;
pub(crate) mod expression_build;
pub(crate) mod multi_bind;
pub(crate) mod parse_values;
pub(crate) mod receiver;
pub(crate) mod types;

pub(crate) use completeness::analyze_branch_exits;
pub(crate) use multi_bind::try_parse_multi_bind_value_block;
pub(crate) use parse_values::{
    complete_retained_then_value, is_missing_produced_value_boundary,
    parse_retained_produced_values,
};
pub(crate) use receiver::try_parse_value_block_at_receiver;
pub(crate) use receiver::{DeferredValueBlock, complete_deferred_value_block};
pub(crate) use types::{
    ActiveValueProductionTarget, ProducedValues, ThenValueContinuation, ThenValueSlot,
    ValueReceiverKind,
};

#[cfg(test)]
#[path = "../tests/value_production_tests.rs"]
mod value_production_tests;
