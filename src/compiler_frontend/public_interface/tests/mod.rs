//! Test root for the focused public-interface test modules.
//!
//! WHAT: wires the focused owner-aligned test modules under one `#[cfg(test)]` root so
//! `public_interface::mod` declares a single `mod tests` entry point. Each child module
//! owns the hidden invariants of one production owner; cross-owner fixtures live in
//! `test_support`.
//! WHY: keeps the public-interface test surface decomposed by production ownership instead
//! of a single mixed-owner monolith, while preserving every test, assertion and fixture.

mod declaration_record_tests;
mod direct_projection_tests;
mod evidence_projection_tests;
mod export_projection_tests;
mod folded_value_tests;
mod interface_validation_tests;
mod local_finalization_tests;
mod test_support;
mod trait_projection_tests;
