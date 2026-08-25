//! Boracle feature lane entry for future borrow-checker work.
//!
//! This module is intentionally isolated behind the `boracle` cargo feature and does
//! not participate in the shipped alpha checker path.

#[cfg(test)]
const BORACLE_FEATURE_MARKER: &str = "boracle";

#[cfg(test)]
mod tests;
