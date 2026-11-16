//! Shared frontend utility helpers.
//!
//! WHAT: cross-cutting helpers for host-path display, Windows verbatim-path comparison
//!       normalization, character classification, token scanning and delimiter-depth
//!       bookkeeping.
//! WHY: these utilities predate some newer subsystem boundaries and are reused across display
//! formatting, parsing and declaration code. `portable_path_display` owns the sole host-path
//! display formatter; filesystem identity and IO stay with their callers.

pub(crate) mod basic;
pub(crate) mod portable_path_display;
pub(crate) mod token_scan;

#[cfg(test)]
mod tests;
