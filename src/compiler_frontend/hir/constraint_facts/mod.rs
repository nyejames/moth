//! Backend-independent constraint facts over final HIR.
//!
//! WHAT: owns the bounded D16 numeric proof policy (`proof`) and the post-lane function analysis
//!       (`analysis`) shared by phase B summary convergence and constraint verification.
//! WHY: the `$safe_math` and `$infallible` guarantees must be computed from one final-HIR walk
//!      after private failure lanes settle. This module is that owner; it never reads backend
//!      proof tables, AST syntax or borrow-summary inference bits.

pub(crate) mod analysis;
pub(crate) mod proof;

pub(crate) use analysis::{
    ConstraintCalleeFacts, ConstraintSummaryBits, analyze_function, function_call_targets,
};
