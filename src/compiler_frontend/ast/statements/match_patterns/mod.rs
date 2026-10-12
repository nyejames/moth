//! Match-pattern parsing and validation.
//!
//! WHAT: parses literal, relational, option-presence, and choice-variant match patterns.
//! WHY: pattern syntax and type validation evolve separately from match arm/body parsing.

mod choice;
mod diagnostics;
mod literal;
mod option;
mod relational;
mod types;

// --------------------------
//  Re-exports
// --------------------------
//
// Public types consumed by AST statement parsing, HIR lowering, and tests.
pub use types::{ChoicePayloadCapture, MatchArm, MatchPattern, RelationalPatternOp};

// pub(super) surface re-exported to the `statements` parent module.
pub(super) use types::ParsedChoicePattern;

pub(super) use choice::{finish_pending_choice_pattern, parse_choice_variant_pattern};
pub(super) use literal::{
    ensure_pattern_compatible_with_subject, finish_retained_pattern, parse_non_choice_pattern,
};
pub(super) use option::{
    OptionCaptureBinding, finish_pending_option_pattern, parse_option_capture_binding,
    parse_option_pattern,
};
pub(crate) use types::MatchSubject;
