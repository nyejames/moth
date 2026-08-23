//! Shared value-production infrastructure for local value-producing blocks.
//!
//! WHAT: owns `then` value parsing, active value targets, all-path exit summaries
//! and reachable produced-value traversal shared by catch handlers and value-producing
//! control flow.
//! WHY: catch-specific fallback parsing and terminal-only enforcement were catch-only
//! artifacts; generalising them now keeps value-block syntax from duplicating the same
//! arity, coercion and completeness logic.
//!
//! This module owns local value production only. It does not own:
//! - result propagation (`!` and `return!`),
//! - option type construction,
//! - tuple user syntax,
//! - template control flow.

pub(crate) mod completeness;
pub(crate) mod multi_bind;
pub(crate) mod parse_values;
pub(crate) mod receiver;
pub(crate) mod types;

pub(crate) use completeness::analyze_branch_exits;
pub(crate) use multi_bind::try_parse_multi_bind_value_block;
pub(crate) use parse_values::{
    ProducedValuesParseInput, is_missing_produced_value_boundary, parse_produced_values_typed,
};
pub(crate) use receiver::try_parse_value_block_at_receiver;
pub(crate) use types::{ActiveValueProductionTarget, ProducedValues, ValueReceiverKind};

#[cfg(test)]
#[path = "../tests/value_production_tests.rs"]
mod value_production_tests;
