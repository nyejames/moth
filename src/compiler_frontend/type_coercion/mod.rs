//! Coercion policy for the Moth compiler frontend.
//!
//! WHAT: Compatibility and coercion policy are centralized here, but assignment-like frontend sites still apply those rules explicitly after parsing.
//! WHY: one policy owner prevents assignment, expression, declaration, return and template sites
//! from drifting.
//!
//! ## Module boundary
//!
//! This module owns:
//! - type-compatibility checks (what values are accepted in what contexts)
//! - contextual coercion at explicit type boundaries (`T` → `T?`; numeric conversion is
//!   explicit-only)
//! - string coercion policy (what is renderable at template boundaries)
//!
//! This module does NOT own:
//! - operator result typing (`eval_expression` owns the exact result-domain policy)
//! - explicit `cast` evidence selection or builtin cast policies (those live in `builtins/casts/`)
//! - template formatting, composition, or slot mechanics

pub(crate) mod compatibility;
pub(crate) mod contextual;
pub(crate) mod parse_context;
pub(crate) mod string;
