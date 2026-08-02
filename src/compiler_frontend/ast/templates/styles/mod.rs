//! Template style formatter/validator modules.
//!
//! WHAT:
//! - Groups frontend-owned formatter implementations used by core and generic built-in directives.
//!
//! WHY:
//! - Keeps frontend style logic modular and separate from generic template parsing.

// -------------------------
//  Public Modules
// -------------------------

pub(crate) mod markdown;
pub(crate) mod raw;
pub(crate) mod whitespace;

#[cfg(test)]
mod tests;
