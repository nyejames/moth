//! Paired compiler-source and public MON parser parity.
//!
//! WHAT: organises source/MON fixtures by syntax family around one shared typed-observation
//! contract.
//! WHY: family files can grow independently without duplicating parser setup, diagnostics or value
//! assertions.

mod calls;
mod choice_spacing;
mod choices;
mod collections;
mod decimals;
mod defaults;
mod float_rounding;
mod gaps;
mod identifiers;
mod integer_destinations;
mod named_entries;
mod nominal;
mod numeric_spelling;
mod perturbations;
mod reserved_names;
mod source_observation;
mod source_only;
mod strings;
mod structure;
mod support;
mod writer;
