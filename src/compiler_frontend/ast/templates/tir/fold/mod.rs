//! Prepared TIR constant folding.
//!
//! The fold entry points and shared reducer state live here as a small local
//! subsystem. `reducer.rs` owns node dispatch and output assembly,
//! `control_flow.rs` owns branch and loop semantics, `wrappers.rs` owns virtual
//! slot and aggregate insertion and `estimate.rs` owns reservation estimates.
//! These consumers deliberately remain TIR-specific rather than becoming a
//! general-purpose TIR visitor.

mod control_flow;
mod estimate;
mod reducer;
mod wrappers;

pub(crate) use reducer::{
    FoldedConstTemplatePiece, fold_prepared_const_template_pattern, fold_prepared_template,
};
