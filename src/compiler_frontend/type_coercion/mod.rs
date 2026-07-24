//! Coercion policy for the Moth compiler frontend.
//!
//! WHAT: Compatibility and coercion policy are centralized here, but assignment-like frontend sites still apply those rules explicitly after parsing.
//! WHY: coercion logic was previously scattered across datatypes, expression
//! evaluation, declarations, returns, and templates — each maintaining its own
//! mini-policy. This module is the single home for all of those decisions.
//!
//! ## Module boundary
//!
//! This module owns:
//! - type-compatibility checks (what values are accepted in what contexts)
//! - contextual coercion at explicit type boundaries (`Int` → `Float`, `T` → `T?`)
//! - string coercion policy (what is renderable at template boundaries)
//!
//! This module does NOT own:
//! - operator result typing (`eval_expression.rs` still decides `Int + Float → Float`)
//! - explicit `cast` evidence selection or builtin cast policies (those live in `builtins/casts/`)
//! - template formatting, composition, or slot mechanics

pub(crate) mod compatibility;
pub(crate) mod contextual;
pub(crate) mod parse_context;
pub(crate) mod string;
