//! Frontend semantic analysis entry points and shared result types.
//!
//! WHAT: exposes the analysis passes that run after HIR construction.
//! WHY: later frontend stages need one place to access borrow-checking reports and analysis data.

pub(crate) mod borrow_checker;
pub(crate) mod numeric_proofs;
