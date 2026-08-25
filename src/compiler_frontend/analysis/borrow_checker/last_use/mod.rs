//! Shared seam for future last-use vocabulary and handoff ownership.
//!
//! The last-use analysis pipeline is intentionally a shared future owner only.
//! It does not own or run the current alpha checker path in this slice.
