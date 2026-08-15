//! Source-backed package root-file identity.
//!
//! WHAT: exposes the shared root/config filename owner and its focused tests.
//! WHY: Stage 0, path resolution and header dependency validation classify root files through one
//! shared policy.

pub(crate) mod root_file;

#[cfg(test)]
#[path = "tests/root_file_tests.rs"]
mod tests;
