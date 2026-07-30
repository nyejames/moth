//! Consumer-local projection of stable provider interface declarations.
//!
//! WHAT: interns canonical provider types, declarations, signatures, defaults, and folded values
//! into one consumer-local AST environment.
//! WHY: imported semantic reconstruction is one focused boundary and does not belong in the
//! general environment orchestration pipeline.

use super::*;

mod callable;
mod canonical;
mod nominal;
mod traits;
mod values;

#[cfg(test)]
pub(crate) use nominal::imported_nominal_path;
