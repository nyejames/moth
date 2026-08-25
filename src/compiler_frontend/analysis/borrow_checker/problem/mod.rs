//! Shared seam for future borrow-problem vocabulary and construction.
//!
//! This is a shared ownership point for later borrowing analyses, but it is not yet
//! implemented or wired into `check_borrows`. The alpha checker remains the only
//! active compiler checker in this slice.
